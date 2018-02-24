use std::ptr;

use gc::Address;
use gc::swiper::crossing::Card;

pub struct CardTable {
    // card table boundaries for old gen (not young gen)
    // table is actually larger but we shouldn't need
    // to touch the rest
    start: Address,
    end: Address,
}

impl CardTable {
    pub fn new(start: Address, end: Address, young_size: usize) -> CardTable {
        // only keep track of card table for old gen,
        // just ignore the card table for the young gen
        let start = start.offset(young_size);

        let card = CardTable {
            start: start,
            end: end,
        };

        // reset card table to all 1's
        card.reset();

        card
    }

    // reset card table entries to 1 (not dirty)
    fn reset(&self) {
        let size = self.end.to_usize() - self.start.to_usize();

        unsafe {
            ptr::write_bytes(self.start.to_mut_ptr::<u8>(), 1, size);
        }
    }

    // visits all dirty cards
    pub fn visit_dirty<F>(&self, mut f: F)
    where
        F: FnMut(Card),
    {
        let mut ptr = self.start;

        while ptr < self.end {
            let val: u8 = unsafe { *ptr.to_ptr() };

            if val == 0 {
                f(Card::from(ptr.offset_from(self.start)));
            }

            ptr = ptr.offset(1);
        }
    }
}
