use std::mem;

/// Packed words with optional runs at either vertical end of the section.
/// The middle keeps its original word boundaries, including widths that do
/// not divide 64, so palette lookups and serialization keep the same order.
#[derive(Debug, Clone)]
pub(super) enum PackedWords {
    Dense(Box<[u64]>),
    Trimmed(Box<TrimmedWords>),
}

#[derive(Debug, Clone)]
pub(super) struct TrimmedWords {
    low: u64,
    high: u64,
    low_count: usize,
    high_count: usize,
    middle: Box<[u64]>,
}

impl PackedWords {
    #[inline]
    pub(super) fn get(&self, index: usize) -> u64 {
        match self {
            Self::Dense(words) => words[index],
            Self::Trimmed(words) => {
                let Some(index) = index.checked_sub(words.low_count) else {
                    return words.low;
                };
                words.middle.get(index).copied().unwrap_or(words.high)
            }
        }
    }

    pub(super) fn dense_mut(&mut self) -> &mut [u64] {
        if let Self::Trimmed(words) = self {
            let len = words.low_count + words.middle.len() + words.high_count;
            let mut dense = Vec::with_capacity(len);
            dense.resize(words.low_count, words.low);
            dense.extend_from_slice(&words.middle);
            dense.resize(len, words.high);
            *self = Self::Dense(dense.into_boxed_slice());
        }
        match self {
            Self::Dense(words) => words,
            Self::Trimmed(_) => unreachable!("the packed words were just expanded"),
        }
    }

    pub(super) fn compact(&mut self) {
        let Self::Dense(words) = self else {
            return;
        };
        // Require enough omitted words to pay for both allocations with room to spare.
        const MIN_SAVED_WORDS: usize = 2 * mem::size_of::<TrimmedWords>() / mem::size_of::<u64>();
        if words.len() <= MIN_SAVED_WORDS {
            return;
        }
        let low = words[0];
        let low_count = words.iter().take_while(|&&word| word == low).count();
        let high = words[words.len() - 1];
        let high_count = words[low_count..]
            .iter()
            .rev()
            .take_while(|&&word| word == high)
            .count();
        if low_count + high_count < MIN_SAVED_WORDS {
            return;
        }
        let middle = words[low_count..words.len() - high_count].into();
        *self = Self::Trimmed(Box::new(TrimmedWords {
            low,
            high,
            low_count,
            high_count,
            middle,
        }));
    }
}
