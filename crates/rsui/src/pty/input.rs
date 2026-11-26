//! Ordered bounded input between terminal callers and a potentially blocked pipe writer.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Condvar, Mutex};

const CAPACITY: usize = 1024 * 1024;
const CHUNK: usize = 16 * 1024;

#[derive(Default)]
struct Pending {
    bytes: VecDeque<u8>,
    closed: bool,
}

#[derive(Default)]
pub(super) struct Input {
    producer: Mutex<()>,
    pending: Mutex<Pending>,
    data: Condvar,
    space: Condvar,
}

impl Input {
    /// Concurrent writes stay contiguous even if a paste is larger than the queue.
    pub(super) fn enqueue(&self, mut bytes: &[u8]) {
        let _turn = self.producer.lock().unwrap();
        let mut pending = self.pending.lock().unwrap();
        while !bytes.is_empty() {
            pending = self.space.wait_while(pending, |p| !p.closed && p.bytes.len() == CAPACITY).unwrap();
            if pending.closed {
                return;
            }
            let count = bytes.len().min(CAPACITY - pending.bytes.len());
            pending.bytes.extend(&bytes[..count]);
            bytes = &bytes[count..];
            self.data.notify_one();
        }
    }

    pub(super) fn run(&self, mut pipe: impl Write) {
        let mut chunk = Vec::new();
        loop {
            let mut pending = self.pending.lock().unwrap();
            pending = self.data.wait_while(pending, |p| !p.closed && p.bytes.is_empty()).unwrap();
            if pending.closed {
                return;
            }
            let count = CHUNK.min(pending.bytes.len());
            chunk.clear();
            chunk.extend(pending.bytes.drain(..count));
            self.space.notify_one();
            drop(pending);
            if pipe.write_all(&chunk).is_err() {
                self.close();
                return;
            }
        }
    }

    pub(super) fn close(&self) {
        let mut pending = self.pending.lock().unwrap();
        pending.closed = true;
        pending.bytes = VecDeque::new();
        self.data.notify_one();
        self.space.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::Duration;

    struct SlowPipe(mpsc::SyncSender<Vec<u8>>);

    impl Write for SlowPipe {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.send(bytes.to_vec()).map_err(|_| io::ErrorKind::BrokenPipe)?;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    #[test]
    fn a_large_paste_is_delivered_in_order_with_bounded_pending_input() {
        let input = Arc::new(Input::default());
        let source = (0..2 * CAPACITY + 1).map(|n| (n % 251) as u8).collect::<Vec<_>>();
        let (tx, rx) = mpsc::sync_channel(0);
        thread::scope(|scope| {
            let worker = scope.spawn(|| input.run(SlowPipe(tx)));
            let producer = scope.spawn(|| input.enqueue(&source));
            let mut received = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(!producer.is_finished());
            let suffix = scope.spawn(|| input.enqueue(b"end"));
            while received.len() < source.len() + 3 {
                received.extend(rx.recv_timeout(Duration::from_secs(5)).unwrap());
            }
            producer.join().unwrap();
            suffix.join().unwrap();
            assert_eq!(&received[..source.len()], &source);
            assert_eq!(&received[source.len()..], b"end");
            input.close();
            worker.join().unwrap();
        });
    }

    #[test]
    fn closing_wakes_a_producer_blocked_on_a_full_queue() {
        let input = Input::default();
        input.enqueue(&vec![0; CAPACITY]);
        let (done, completion) = mpsc::channel();
        thread::scope(|scope| {
            scope.spawn(|| { input.enqueue(b"more"); done.send(()).unwrap(); });
            assert!(completion.recv_timeout(Duration::from_millis(30)).is_err());
            input.close();
            completion.recv_timeout(Duration::from_secs(2)).unwrap();
        });
    }
}
