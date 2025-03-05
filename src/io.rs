use actix_web::web::Bytes;
use linked_hash_map::LinkedHashMap;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use rand::{distributions::Alphanumeric, thread_rng, Rng, seq::SliceRandom};
use std::cell::RefCell;

pub type PasteStore = RwLock<LinkedHashMap<String, Bytes>>;

static BUFFER_SIZE: Lazy<usize> = Lazy::new(|| argh::from_env::<crate::BinArgs>().buffer_size);

/// Ensures `ENTRIES` is less than the size of `BIN_BUFFER_SIZE`. If it isn't then
/// `ENTRIES.len() - BIN_BUFFER_SIZE` elements will be popped off the front of the map.
///
/// During the purge, `ENTRIES` is locked and the current thread will block.
fn purge_old(entries: &mut LinkedHashMap<String, Bytes>) {
    if entries.len() > *BUFFER_SIZE {
        let to_remove = entries.len() - *BUFFER_SIZE;

        for _ in 0..to_remove {
            entries.pop_front();
        }
    }
}

/// Generates a random ID with 2 numbers and 1 letter in random positions
pub fn generate_id() -> String {
    let mut rng = thread_rng();
    
    // Generate 2 random digits
    let digit1 = rng.gen_range(0..10).to_string();
    let digit2 = rng.gen_range(0..10).to_string();
    
    // Generate 1 random letter (lowercase or uppercase)
    let letter = if rng.gen_bool(0.5) {
        // Lowercase letter
        (rng.gen_range(b'a'..=b'z') as char).to_string()
    } else {
        // Uppercase letter
        (rng.gen_range(b'A'..=b'Z') as char).to_string()
    };
    
    // Create the characters and shuffle them
    let mut chars = vec![digit1, digit2, letter];
    chars.shuffle(&mut rng);
    
    // Join the characters into a single string
    chars.join("")
}

/// Stores a paste under the given id
pub fn store_paste(entries: &PasteStore, id: String, content: Bytes) {
    let mut entries = entries.write();

    purge_old(&mut entries);

    entries.insert(id, content);
}

/// Get a paste by id.
///
/// Returns `None` if the paste doesn't exist.
pub fn get_paste(entries: &PasteStore, id: &str) -> Option<Bytes> {
    // need to box the guard until owning_ref understands Pin is a stable address
    entries.read().get(id).map(Bytes::clone)
}
