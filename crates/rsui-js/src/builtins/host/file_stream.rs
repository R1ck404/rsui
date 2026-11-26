//! File streams hold one reader across pulls. Opening, seeking and each bounded read run on a worker.

use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::background::Outcome;
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

#[derive(Default)]
pub(crate) struct Readers {
    next: u32,
    open: HashMap<u32, Arc<Mutex<Reader>>>,
}

struct Reader {
    path: String,
    start: u64,
    input: Option<Box<dyn Read + Send>>,
}

impl Reader {
    fn open(&self) -> io::Result<Box<dyn Read + Send>> {
        if let Some(packed) = crate::packed::open_packed_reader(Path::new(&self.path), self.start) {
            return packed;
        }
        let mut file = std::fs::File::open(&self.path)?;
        if self.start > 0 {
            file.seek(SeekFrom::Start(self.start))?;
        }
        Ok(Box::new(file))
    }

    fn read(&mut self, length: usize) -> Outcome {
        if self.input.is_none() {
            match self.open() {
                Ok(file) => self.input = Some(file),
                Err(error) => return super::failed(&error, "open", self.path.clone()),
            }
        }
        let mut bytes = vec![0; length];
        match self.input.as_mut().unwrap().read(&mut bytes) {
            Ok(count) => {
                bytes.truncate(count);
                Outcome::Bytes(bytes)
            }
            Err(error) => super::failed(&error, "read", self.path.clone()),
        }
    }
}

pub(super) fn create(vm: &mut Vm, args: Args) -> Run<Value> {
    let path = super::text_argument(vm, &args, 0)?;
    let start = vm.arg(&args, 1);
    let start = vm.to_number(start)? as u64;
    let readers = &mut vm.file_streams;
    while readers.open.contains_key(&readers.next) {
        readers.next = readers.next.wrapping_add(1);
    }
    let id = readers.next;
    readers.next = readers.next.wrapping_add(1);
    readers.open.insert(id, Arc::new(Mutex::new(Reader { path, start, input: None })));
    Ok(Value::number(f64::from(id)))
}

pub(super) fn read(vm: &mut Vm, args: Args) -> Run<Value> {
    let id = vm.arg(&args, 0);
    let id = vm.to_number(id)? as u32;
    let length = vm.arg(&args, 1);
    // A high water mark controls buffering. An exceptionally large one needn't allocate it all in a single read.
    let length = (vm.to_number(length)? as usize).min(1024 * 1024);
    let Some(reader) = vm.file_streams.open.get(&id).cloned() else {
        return Err(vm.type_error("file stream is closed"));
    };
    Ok(vm.in_background(move || reader.lock().unwrap().read(length)))
}

pub(super) fn close(vm: &mut Vm, args: Args) -> Run<Value> {
    let id = vm.arg(&args, 0);
    let id = vm.to_number(id)? as u32;
    vm.file_streams.open.remove(&id);
    Ok(Value::UNDEFINED)
}
