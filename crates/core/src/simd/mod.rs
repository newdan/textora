//! Provides various high-throughput utilities.

pub mod lines_bwd;
pub mod lines_fwd;
mod memchr2;

pub use lines_bwd::*;
pub use lines_fwd::*;
pub use memchr2::*;

#[cfg(test)]
mod test {
    // Knuth's MMIX LCG
    pub fn make_rng() -> impl FnMut() -> usize {
        let mut state = 1442695040888963407u64;
        move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            state as usize
        }
    }

    pub fn generate_random_text(len: usize) -> String {
        const ALPHABET: &[u8; 20] = b"0123456789abcdef\n\n\n\n";

        let mut rng = make_rng();
        let mut res = String::new();

        for _ in 0..len {
            res.push(ALPHABET[rng() % ALPHABET.len()] as char);
        }

        res
    }

    pub fn count_lines(text: &str) -> usize {
        text.lines().count()
    }

    #[test]
    fn dispatch_initialization_is_thread_safe() {
        let input = std::sync::Arc::new(b"alpha\nbeta\r\ngamma".to_vec());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let input = input.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..10_000 {
                        assert_eq!(super::memchr2(b'\n', b'\r', &input, 0), 5);
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    }
}
