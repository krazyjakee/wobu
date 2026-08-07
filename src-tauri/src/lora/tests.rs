use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Instant;

const LARGE_IMAGE_BYTES: usize = 8 * 1024 * 1024;

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("wobu-{label}-{}", wobu_core::new_id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct RepeatedBytes {
    byte: u8,
    remaining: usize,
    largest_request: Arc<AtomicUsize>,
}

impl io::Read for RepeatedBytes {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.largest_request.fetch_max(buffer.len(), Ordering::SeqCst);
        let read = self.remaining.min(buffer.len());
        buffer[..read].fill(self.byte);
        self.remaining -= read;
        Ok(read)
    }
}

fn repeated_hash(byte: u8, len: usize) -> String {
    let mut hasher = blake3::Hasher::new();
    let block = [byte; STAGING_BUFFER_BYTES];
    let mut remaining = len;
    while remaining > 0 {
        let take = remaining.min(block.len());
        hasher.update(&block[..take]);
        remaining -= take;
    }
    hasher.finalize().to_hex().to_string()
}

#[test]
fn large_dataset_is_opened_once_per_source_and_streamed_through_one_small_buffer() {
    let root = TestDir::new("lora-large-dataset");
    let stage = root.0.join("stage");
    let mut bytes_by_id = HashMap::new();
    let inputs = (0..REQUIRED_IMAGES)
        .map(|index| {
            let asset_id = wobu_core::new_id();
            let byte = u8::try_from(index + 1).unwrap();
            let source = root.0.join(format!("source-{index}.png"));
            let file = std::fs::File::create(&source).unwrap();
            file.set_len(LARGE_IMAGE_BYTES as u64).unwrap();
            bytes_by_id.insert(asset_id, byte);
            TrainingInput {
                asset_id,
                hash: repeated_hash(byte, LARGE_IMAGE_BYTES),
                source,
                bytes: LARGE_IMAGE_BYTES as u64,
            }
        })
        .collect();
    let request = StageRequest {
        subject_id: wobu_core::new_id(),
        subject_name: "Kael".into(),
        model: "flux-dev".into(),
        model_family: "flux".into(),
        inputs,
        max_inputs: REQUIRED_IMAGES,
    };
    let largest_request = Arc::new(AtomicUsize::new(0));
    let mut opens = HashMap::<Id, usize>::new();

    let staged = stage_training_with(&stage, request, "wobu_kael", &Cancel::new(), |input| {
        *opens.entry(input.asset_id).or_default() += 1;
        Ok(Box::new(RepeatedBytes {
            byte: bytes_by_id[&input.asset_id],
            remaining: LARGE_IMAGE_BYTES,
            largest_request: Arc::clone(&largest_request),
        }))
    });
    let staged = match staged {
        Ok(staged) => staged,
        Err(_) => panic!("the synthetic large dataset should stage"),
    };

    assert_eq!(opens.len(), REQUIRED_IMAGES);
    assert!(opens.values().all(|count| *count == 1));
    assert_eq!(staged.input_hashes.len(), REQUIRED_IMAGES);
    assert_eq!(largest_request.load(Ordering::SeqCst), STAGING_BUFFER_BYTES);
    let staged_bytes: u64 = std::fs::read_dir(stage.join("inputs"))
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum();
    assert_eq!(staged_bytes, (REQUIRED_IMAGES * LARGE_IMAGE_BYTES) as u64);
}

struct CancelledRead {
    started: Option<mpsc::Sender<()>>,
    cancel: Cancel,
}

impl io::Read for CancelledRead {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if let Some(started) = self.started.take() {
            let _ = started.send(());
        }
        while !self.cancel.is_cancelled() {
            std::thread::yield_now();
        }
        buffer[0] = 1;
        Ok(1)
    }
}

#[test]
fn cancellation_during_a_source_copy_removes_the_private_dataset() {
    let root = TestDir::new("lora-cancel-stage");
    let stage = root.0.join("stage");
    let source = root.0.join("source.png");
    std::fs::write(&source, [0]).unwrap();
    let input = TrainingInput {
        asset_id: wobu_core::new_id(),
        hash: repeated_hash(1, 1),
        source,
        bytes: 1,
    };
    let request = StageRequest {
        subject_id: wobu_core::new_id(),
        subject_name: "Kael".into(),
        model: "flux-dev".into(),
        model_family: "flux".into(),
        inputs: vec![input],
        max_inputs: REQUIRED_IMAGES,
    };
    let cancel = Cancel::new();
    let worker_cancel = cancel.clone();
    let worker_stage = stage.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let started_at = Instant::now();
    let worker = std::thread::spawn(move || {
        let reader_cancel = worker_cancel.clone();
        let mut started_tx = Some(started_tx);
        stage_training_with(&worker_stage, request, "wobu_kael", &worker_cancel, move |_| {
            Ok(Box::new(CancelledRead {
                started: started_tx.take(),
                cancel: reader_cancel.clone(),
            }))
        })
    });

    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    cancel.cancel();
    assert!(matches!(worker.join().unwrap(), Err(StageError::Cancelled)));
    assert!(!stage.exists(), "a cancelled dataset must clean up its staging folder");
    assert!(started_at.elapsed() < Duration::from_secs(5));
}
