use super::super::{
    Compiler, TokenStream, ident, identifier,
    model::{
        CompileCondition, DamageSourcePredicate, EntityPredicate, EntityTarget, StateProperties,
    },
    quote,
};
use serde::Deserialize;
use serde::de::IgnoredAny;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KilledByPlayer {}

impl CompileCondition for KilledByPlayer {
    fn compile(&self, _compiler: &mut Compiler) -> TokenStream {
        quote! { ctx.killed_by_player }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityProperties {
    // TODO: Accept an absent predicate with vanilla's unconditional success.
    pub entity: EntityTarget,
    pub predicate: EntityPredicate,
}

impl CompileCondition for EntityProperties {
    fn compile(&self, _compiler: &mut Compiler) -> TokenStream {
        let Self {
            entity, predicate, ..
        } = self;
        let target = match entity {
            EntityTarget::This => quote! { LootContextEntity::This },
            EntityTarget::Attacker => quote! { LootContextEntity::Killer },
            EntityTarget::DirectAttacker => quote! { LootContextEntity::DirectKiller },
            EntityTarget::LastDamagePlayer => quote! { LootContextEntity::KillerPlayer },
            EntityTarget::InteractingEntity => quote! { LootContextEntity::Interacting },
        };
        let predicate = Compiler::entity_predicate(predicate);
        quote! { match ctx.get_entity(#target) { Some(entity) => { let _ = &entity; #predicate }, None => false } }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DamageSourceProperties {
    // TODO: Accept an absent predicate while requiring origin and damage source.
    pub predicate: Box<DamageSourcePredicate>,
}

impl CompileCondition for DamageSourceProperties {
    fn compile(&self, _compiler: &mut Compiler) -> TokenStream {
        let Self { predicate, .. } = self;
        let predicate = Compiler::damage_source_predicate(predicate);
        quote! { ctx.origin.is_some() && (#predicate) }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationCheck {
    #[serde(default, rename = "offsetX")]
    offset_x: i32,
    #[serde(default, rename = "offsetY")]
    offset_y: i32,
    #[serde(default, rename = "offsetZ")]
    offset_z: i32,
    predicate: Option<LocationPredicate>,
}

#[derive(Deserialize)]
struct LocationPredicate {
    block: Option<LocationBlock>,
    biomes: Option<OneOrMany>,
    #[serde(flatten)]
    unsupported: BTreeMap<String, IgnoredAny>,
}

#[derive(Deserialize)]
struct LocationBlock {
    blocks: Option<OneOrMany>,
    #[serde(default)]
    state: StateProperties,
    #[serde(flatten)]
    unsupported: BTreeMap<String, IgnoredAny>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn values(&self) -> &[String] {
        match self {
            Self::One(value) => std::slice::from_ref(value),
            Self::Many(values) => values,
        }
    }
}

impl CompileCondition for LocationCheck {
    fn compile(&self, _compiler: &mut Compiler) -> TokenStream {
        let Some(predicate) = &self.predicate else {
            return quote! { ctx.origin.is_some() };
        };
        let x = self.offset_x;
        let y = self.offset_y;
        let z = self.offset_z;
        let mut checks = Vec::new();
        if let Some(block) = &predicate.block {
            let blocks = block
                .blocks
                .as_ref()
                .map(|blocks| {
                    let alternatives = blocks.values().iter().map(|key| {
                    if let Some(tag) = key.strip_prefix('#') {
                        let tag = identifier(tag);
                        quote! { REGISTRY.blocks.is_in_tag(state.get_block(), &#tag) }
                    } else {
                        let block = ident(key);
                        quote! { state.get_block() == &steel_registry::vanilla_blocks::#block }
                    }
                }).collect::<Vec<_>>();
                    Compiler::any(&alternatives)
                })
                .unwrap_or_else(|| quote! { true });
            let properties = block
                .state
                .iter()
                .map(|(name, expected)| {
                    quote! { state.get_property_str(#name).is_some_and(|value| value == #expected) }
                })
                .collect::<Vec<_>>();
            let extras = block
                .unsupported
                .keys()
                .map(|key| {
                    let operation = format!("location block predicate {key}");
                    quote! { ctx.unsupported::<bool>(#operation)? }
                })
                .collect::<Vec<_>>();
            checks.push(quote! {
                world.chunk_map.with_full_chunk(steel_utils::ChunkPos::from_block_pos(pos), |chunk| {
                    let state = chunk.get_block_state(pos);
                    (#blocks) #(&& #properties)*
                }).unwrap_or(false) #(&& #extras)*
            });
        }
        if let Some(biomes) = &predicate.biomes {
            let alternatives = biomes
                .values()
                .iter()
                .map(|key| {
                    if let Some(tag) = key.strip_prefix('#') {
                        let tag = identifier(tag);
                        quote! { REGISTRY.biomes.is_in_tag(biome, &#tag) }
                    } else {
                        let key = identifier(key);
                        quote! { biome.key == #key }
                    }
                })
                .collect::<Vec<_>>();
            let matches = Compiler::any(&alternatives);
            checks.push(quote! { world.biome_at(pos).is_some_and(|biome| #matches) });
        }
        for key in predicate.unsupported.keys() {
            let operation = format!("location predicate {key}");
            checks.push(quote! { ctx.unsupported::<bool>(#operation)? });
        }
        let matches = Compiler::all(&checks);
        quote! {
            match (ctx.origin, ctx.world) {
                (Some(origin), Some(world)) => {
                    let pos = steel_utils::BlockPos::containing(
                        origin.x + f64::from(#x), origin.y + f64::from(#y), origin.z + f64::from(#z),
                    );
                    #matches
                }
                _ => false,
            }
        }
    }
}
