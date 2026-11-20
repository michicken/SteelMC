use super::{
    Arc, Chunk, ChunkHolder, ChunkMap, ChunkSaveDependency, ChunkStatus, ChunkStorage,
    ClearedBlockEntities, FinalizedBlockEntityUnload, FxHashSet, instrument, io, mem,
};
use crate::chunk_saver::{PreparedChunkSave, RegionManager};
use tokio::sync::oneshot;

enum UnloadSave {
    Encoded {
        status: ChunkStatus,
        compressed: Vec<u8>,
    },
    Ram(Box<PreparedChunkSave>),
}

impl ChunkMap {
    fn prepare_unload_snapshot(&self, holder: &Arc<ChunkHolder>) -> Option<PreparedChunkSave> {
        let save_preparation = holder.try_begin_save_preparation()?;
        // Vanilla only persists chunks once they reach StructureStarts.
        let chunk = holder.try_chunk(ChunkStatus::StructureStarts)?;
        let status = holder.published_status()?;
        let world = self.world_gen_context.world();
        let chunk_pos = holder.get_pos();
        let runtime_entities = world
            .entity_manager()
            .get_saveable_entities_for_chunk(chunk_pos);
        let force = world.entity_manager().has_save_pending_for_chunk(chunk_pos);
        let dirty = chunk.take_dirty();
        let prepared = if dirty || force {
            ChunkStorage::prepare_chunk_save(chunk, status, &runtime_entities, true)
        } else {
            None
        };
        let prepared = save_preparation.finish(prepared).flatten();
        if prepared.is_none() && dirty {
            chunk.mark_dirty();
        }
        prepared
    }

    async fn prepare_unload_save(
        self: &Arc<Self>,
        holder: &Arc<ChunkHolder>,
    ) -> io::Result<Option<(UnloadSave, Vec<i32>)>> {
        let (sender, receiver) = oneshot::channel();
        let map = Arc::clone(self);
        let holder = Arc::clone(holder);
        self.chunk_encoding_pool.spawn(move || {
            let result = map
                .prepare_unload_snapshot(&holder)
                .map(|mut prepared| {
                    let handled_entities = mem::take(&mut prepared.handled_runtime_entity_ids);
                    // Encode in the same pool job so queued work has no uncompressed snapshot.
                    let save = match map.storage.as_ref() {
                        ChunkStorage::Disk(_) => RegionManager::encode_chunk(&prepared.persistent)
                            .map(|compressed| UnloadSave::Encoded {
                                status: prepared.status,
                                compressed,
                            })?,
                        ChunkStorage::RamOnly(_) => UnloadSave::Ram(Box::new(prepared)),
                    };
                    Ok((save, handled_entities))
                })
                .transpose();
            if let Err(unsent) = sender.send(result)
                && !matches!(unsent, Ok(None))
            {
                Self::mark_chunk_dirty_for_save_retry(&holder);
            }
        });
        receiver.await.map_err(|_| {
            io::Error::other("chunk save preparation task ended without returning a result")
        })?
    }

    /// Saves a chunk to disk. Does not remove from `unloading_chunks`.
    #[instrument(level = "trace", skip(self, chunk_holder, _save_dependency), fields(chunk = ?chunk_holder.get_pos()))]
    pub(super) async fn save_chunk(
        self: &Arc<Self>,
        chunk_holder: &Arc<ChunkHolder>,
        _save_dependency: ChunkSaveDependency,
    ) {
        let chunk_pos = chunk_holder.get_pos();
        self.flush_queued_light_changes_touching_chunk_for_save(chunk_pos)
            .await;

        let result = self.prepare_unload_save(chunk_holder).await;
        let (save, handled_entities) = match result {
            Ok(Some(prepared)) => prepared,
            Ok(None) => return,
            Err(error) => {
                tracing::error!(chunk = ?chunk_pos, "Error preparing chunk save: {error}");
                Self::mark_chunk_dirty_for_save_retry(chunk_holder);
                return;
            }
        };

        let result = match (self.storage.as_ref(), save) {
            (ChunkStorage::Disk(region), UnloadSave::Encoded { status, compressed }) => {
                region
                    .write_encoded_chunk(chunk_pos, status, &compressed)
                    .await
            }
            (ChunkStorage::RamOnly(ram), UnloadSave::Ram(prepared)) => {
                ram.save_chunk_data(*prepared).await
            }
            _ => Err(io::Error::other(
                "chunk save prepared for a different storage backend",
            )),
        };
        match result {
            Ok(true) => self
                .world_gen_context
                .world()
                .entity_manager()
                .on_chunk_saved(chunk_pos, &handled_entities),
            Ok(false) => Self::mark_chunk_dirty_for_save_retry(chunk_holder),
            Err(error) => {
                tracing::error!(chunk = ?chunk_pos, "Error saving chunk: {error}");
                Self::mark_chunk_dirty_for_save_retry(chunk_holder);
            }
        }
    }

    pub(super) fn mark_chunk_dirty_for_save_retry(chunk_holder: &ChunkHolder) {
        let Some(chunk) = chunk_holder.try_chunk(ChunkStatus::StructureStarts) else {
            return;
        };
        chunk.mark_dirty();
    }

    /// Processes chunks that are pending unload.
    ///
    /// Iterates over `unloading_chunks`. For each chunk with `strong_count == 1`:
    /// - If dirty: spawn save task (keep until saved and clean)
    /// - If not dirty: release region handle and remove
    #[instrument(level = "trace", skip(self))]
    pub(super) fn process_unloads(self: &Arc<Self>) {
        self.propagate_queued_light_changes();

        let mut finalized = Vec::new();
        {
            let light_updates = self.light_updates.lock();
            self.unloading_chunks.retain_sync(|pos, holder| {
                if light_updates.touches_chunk(*pos) {
                    return true;
                }

                if Arc::strong_count(holder) != 1 {
                    return true;
                }

                let is_dirty = holder
                    .try_chunk(ChunkStatus::StructureStarts)
                    .is_some_and(Chunk::is_dirty);
                let has_save_pending_entities = self
                    .world_gen_context
                    .world()
                    .entity_manager()
                    .has_save_pending_for_chunk(*pos);

                if is_dirty || has_save_pending_entities {
                    let save_dependency = holder.add_save_dependency();
                    let holder_clone = Arc::clone(holder);
                    let map_clone = Arc::clone(self);
                    self.task_tracker.spawn_on(
                        async move {
                            map_clone.save_chunk(&holder_clone, save_dependency).await;
                        },
                        self.chunk_runtime.handle(),
                    );
                    return true;
                }

                let has_chunk = holder.try_chunk(ChunkStatus::Empty).is_some();
                finalized.push((*pos, Arc::clone(holder), has_chunk));
                false
            });
        }

        let world = self.world_gen_context.world();
        for (pos, holder, has_chunk) in finalized {
            let cleared = if has_chunk {
                holder.try_chunk(ChunkStatus::Empty).map_or_else(
                    ClearedBlockEntities::default,
                    |chunk| {
                        if let Some(full) = holder.try_full_chunk() {
                            full.clear_all_block_entities_staged()
                        } else {
                            chunk.clear_all_block_entities();
                            ClearedBlockEntities::default()
                        }
                    },
                )
            } else {
                ClearedBlockEntities::default()
            };
            self.finalized_block_entity_unloads
                .lock()
                .push(FinalizedBlockEntityUnload {
                    holder: Arc::clone(&holder),
                    lifecycle_dispatchers: cleared.lifecycle_dispatchers,
                    positions: cleared.positions,
                });

            world.unregister_full_chunk_ticks(pos);
            world.on_entity_chunk_unload_finalized(pos);
            if has_chunk {
                let map_clone = Arc::clone(self);
                self.task_tracker.spawn_on(
                    async move {
                        if let Err(e) = map_clone.storage.release_chunk(pos).await {
                            tracing::error!(?pos, "Error releasing chunk: {e}");
                        }
                    },
                    self.chunk_runtime.handle(),
                );
            }
        }
    }

    /// Saves all dirty chunks to disk.
    ///
    /// This method should be called during graceful shutdown to ensure all
    /// modified chunks are persisted. It saves:
    /// 1. All dirty chunks in the active `chunks` map
    /// 2. All chunks pending unload in the `unloading_chunks` map
    /// 3. Closes all region file handles (flushing headers)
    ///
    /// Returns the number of chunks saved.
    #[instrument(level = "info", skip(self), name = "save_all_chunks")]
    #[expect(
        clippy::too_many_lines,
        reason = "shutdown persistence keeps chunk coverage and entity auditing in one pass"
    )]
    pub async fn save_all_chunks(self: &Arc<Self>) -> io::Result<usize> {
        let mut saved_count = 0;

        self.flush_queued_light_changes_for_save().await;

        // Collect all chunks from both maps
        let all_chunks: Vec<Arc<ChunkHolder>> = {
            let mut chunks = Vec::new();
            self.chunks.iter_sync(|_, holder| {
                chunks.push(Arc::clone(holder));
                true
            });
            self.unloading_chunks.iter_sync(|_, holder| {
                chunks.push(Arc::clone(holder));
                true
            });
            chunks
        };
        let mut covered_chunk_positions = FxHashSet::default();

        tracing::info!(chunk_count = all_chunks.len(), "Saving chunks");

        // Save all chunks that have data
        for holder in &all_chunks {
            let chunk_pos = holder.get_pos();
            let prepared = {
                let Some(chunk) = holder.try_chunk(ChunkStatus::StructureStarts) else {
                    // Matches save_chunk: StructureStarts is the first persisted
                    // chunk status, so lower-status chunks do not own durable
                    // runtime entity data.
                    continue;
                };
                let Some(status) = holder.published_status() else {
                    continue;
                };
                let world = self.world_gen_context.world();
                let runtime_entities = world
                    .entity_manager()
                    .get_saveable_entities_for_chunk(chunk_pos);
                let force = world.entity_manager().has_save_pending_for_chunk(chunk_pos);
                let dirty = chunk.take_dirty();
                let prepared = if dirty || force {
                    ChunkStorage::prepare_chunk_save(chunk, status, &runtime_entities, true)
                } else {
                    None
                };
                let Some(prepared) = prepared else {
                    if dirty {
                        chunk.mark_dirty();
                    } else if !force {
                        covered_chunk_positions.insert(chunk_pos);
                    }
                    continue;
                };
                prepared
            };

            let mut prepared = prepared;
            let handled_runtime_entity_ids = mem::take(&mut prepared.handled_runtime_entity_ids);
            let world = self.world_gen_context.world();
            let _save_dependency = holder.add_save_dependency();
            match self
                .storage
                .save_chunk_data(prepared, self.chunk_encoding_pool.as_ref())
                .await
            {
                Ok(true) => {
                    world
                        .entity_manager()
                        .on_chunk_saved(chunk_pos, &handled_runtime_entity_ids);
                    covered_chunk_positions.insert(chunk_pos);
                    saved_count += 1;
                }
                Ok(false) => Self::mark_chunk_dirty_for_save_retry(holder),
                Err(e) => {
                    tracing::error!(chunk = ?holder.get_pos(), "Failed to save chunk: {e}");
                    Self::mark_chunk_dirty_for_save_retry(holder);
                }
            }
        }

        let world = self.world_gen_context.world();
        let covered_chunk_positions = covered_chunk_positions.into_iter().collect::<Vec<_>>();
        let unsaved_entities = world
            .entity_manager()
            .saveable_entities_outside_chunks(&covered_chunk_positions);
        if !unsaved_entities.is_empty() {
            let chunk_count = unsaved_entities
                .iter()
                .map(|entity| entity.chunk)
                .collect::<FxHashSet<_>>()
                .len();
            let sample = unsaved_entities
                .iter()
                .take(16)
                .map(|entity| format!("{}:{}@{:?}", entity.entity_id, entity.uuid, entity.chunk))
                .collect::<Vec<_>>()
                .join(", ");
            tracing::warn!(
                entity_count = unsaved_entities.len(),
                chunk_count,
                sample = %sample,
                "Saveable runtime entities remain in chunks without save holders after chunk save"
            );
        }

        // Close all region files (flushes headers and releases file handles)
        if let Err(e) = self.storage.close_all().await {
            tracing::error!("Failed to close region files: {e}");
        }

        tracing::info!(
            saved_count,
            total_checked = all_chunks.len(),
            "Chunk save complete"
        );

        Ok(saved_count)
    }
}
