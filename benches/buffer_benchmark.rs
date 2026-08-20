use std::hint::black_box;
use std::time::Instant;
use rust_expander::buffer::KeyBuffer;

fn main() {
    println!("============================================================");
    println!("       Win-ARM Text Expander - Performance Benchmark        ");
    println!("============================================================");

    let iterations = 1_000_000;

    // Benchmark 1: ends_with with non-matching triggers (99.9% of all keystrokes)
    {
        let mut buf = KeyBuffer::new(64);
        for ch in "This is a quick test sentence typed by the user".chars() {
            buf.push(ch);
        }

        let triggers = [
            ":email",
            ":sig",
            ":addr",
            ":phone",
            ":date",
            ":br",
            ":shrug",
            ":react",
            ":todo",
            ":meeting",
        ];

        // Warmup
        for _ in 0..10_000 {
            for trigger in &triggers {
                black_box(buf.ends_with(black_box(trigger)));
            }
        }

        let start = Instant::now();
        for _ in 0..iterations {
            for trigger in &triggers {
                black_box(buf.ends_with(black_box(trigger)));
            }
        }
        let elapsed = start.elapsed();
        let total_calls = iterations * triggers.len();
        let per_call_ns = elapsed.as_nanos() as f64 / total_calls as f64;

        println!(
            "Benchmark: ends_with (non-matching, 10 snippets scan)\n  Total calls: {}\n  Elapsed: {:?}\n  Average per call: {:.2} ns\n",
            total_calls, elapsed, per_call_ns
        );
    }

    // Benchmark 2: ends_with with matching trigger (expansion triggered)
    {
        let mut buf = KeyBuffer::new(64);
        for ch in "Hello my email is :email".chars() {
            buf.push(ch);
        }

        let trigger = ":email";

        // Warmup
        for _ in 0..10_000 {
            black_box(buf.ends_with(black_box(trigger)));
        }

        let start = Instant::now();
        for _ in 0..iterations {
            black_box(buf.ends_with(black_box(trigger)));
        }
        let elapsed = start.elapsed();
        let per_call_ns = elapsed.as_nanos() as f64 / iterations as f64;

        println!(
            "Benchmark: ends_with (exact match ':email')\n  Total calls: {}\n  Elapsed: {:?}\n  Average per call: {:.2} ns\n",
            iterations, elapsed, per_call_ns
        );
    }

    // Benchmark 3: push characters into circular buffer
    {
        let mut buf = KeyBuffer::new(64);
        let sample = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

        // Warmup
        for _ in 0..10_000 {
            for ch in sample.chars() {
                buf.push(ch);
            }
        }

        let start = Instant::now();
        for _ in 0..iterations {
            for ch in sample.chars() {
                buf.push(black_box(ch));
            }
        }
        let elapsed = start.elapsed();
        let total_pushes = iterations * sample.len();
        let per_call_ns = elapsed.as_nanos() as f64 / total_pushes as f64;

        println!(
            "Benchmark: KeyBuffer::push\n  Total pushes: {}\n  Elapsed: {:?}\n  Average per push: {:.2} ns\n",
            total_pushes, elapsed, per_call_ns
        );
    }

    println!("============================================================");
}
