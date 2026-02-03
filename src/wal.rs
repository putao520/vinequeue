use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use serde::de::DeserializeOwned;
use serde::Serialize;
use crate::{Config, QueueError, WalSyncMode};
const WAL_MAGIC: u32 = 0x5649_4E45; // 'VINE'
const WAL_VERSION: u16 = 1;
const WAL_COMPRESSION_SNAPPY: u8 = 1;
const HEADER_SIZE: usize = 4 + 2 + 1 + 1 + 8 + 4;
struct WalInner {
    path: PathBuf,
    file: File,
    record_count: u64,
    max_size: u64,
    sync_mode: WalSyncMode,
    sync_interval: Duration,
    last_sync: Instant,
    current_size: u64,
}

pub struct Wal<T> {
    inner: Mutex<WalInner>,
    _marker: std::marker::PhantomData<T>,
}
impl<T> Wal<T>
where
    T: Serialize + DeserializeOwned,
{
    pub fn open(config: &Config) -> Result<Self, QueueError> {
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&config.wal_path)?;

        let metadata = file.metadata()?;
        let current_size = metadata.len();

        let record_count = if current_size >= HEADER_SIZE as u64 {
            let mut header_bytes = [0u8; HEADER_SIZE];
            file.seek(SeekFrom::Start(0))?;
            file.read_exact(&mut header_bytes)?;
            parse_header(&header_bytes)?
        } else {
            write_header(&mut file, 0)?;
            file.flush()?;
            0
        };

        Ok(Self {
            inner: Mutex::new(WalInner {
                path: config.wal_path.clone(),
                file,
                record_count,
                max_size: config.wal_max_size,
                sync_mode: config.wal_sync_mode,
                sync_interval: config.flush_timeout,
                last_sync: Instant::now(),
                current_size: current_size.max(HEADER_SIZE as u64),
            }),
            _marker: std::marker::PhantomData,
        })
    }

    pub fn write(&self, item: &T) -> Result<(), QueueError> {
        let mut inner = self.inner.lock().map_err(|_| QueueError::WalWriteFailed)?;
        let payload = encode_item(item)?;
        let record = build_record(inner.record_count + 1, payload)?;

        if inner.current_size + record.len() as u64 > inner.max_size {
            return Err(QueueError::WalWriteFailed);
        }

        inner.file.seek(SeekFrom::End(0))?;
        inner.file.write_all(&record)?;
        inner.file.flush()?;
        inner.current_size += record.len() as u64;
        inner.record_count += 1;
        let record_count = inner.record_count;
        write_header(&mut inner.file, record_count)?;
        maybe_sync(&mut inner)?;
        Ok(())
    }

    pub fn write_batch(&self, items: &[T]) -> Result<(), QueueError> {
        let mut inner = self.inner.lock().map_err(|_| QueueError::WalWriteFailed)?;
        let mut buffer = Vec::new();
        let mut new_count = inner.record_count;
        for item in items {
            let payload = encode_item(item)?;
            let record = build_record(new_count + 1, payload)?;
            new_count += 1;
            buffer.extend_from_slice(&record);
        }

        if inner.current_size + buffer.len() as u64 > inner.max_size {
            return Err(QueueError::WalWriteFailed);
        }

        inner.file.seek(SeekFrom::End(0))?;
        inner.file.write_all(&buffer)?;
        inner.file.flush()?;
        inner.current_size += buffer.len() as u64;
        inner.record_count = new_count;
        let record_count = inner.record_count;
        write_header(&mut inner.file, record_count)?;
        maybe_sync(&mut inner)?;
        Ok(())
    }

    pub fn read_all(&self) -> Result<Vec<T>, QueueError> {
        let inner = self.inner.lock().map_err(|_| QueueError::WalWriteFailed)?;
        read_records(&inner.file)
    }

    pub fn read_and_clear(&self) -> Result<Vec<T>, QueueError> {
        let mut inner = self.inner.lock().map_err(|_| QueueError::WalWriteFailed)?;
        let records = read_records(&inner.file)?;
        clear_file(&mut inner.file)?;
        inner.record_count = 0;
        inner.current_size = HEADER_SIZE as u64;
        inner.last_sync = Instant::now();
        Ok(records)
    }

    pub fn clear(&self) -> Result<(), QueueError> {
        let mut inner = self.inner.lock().map_err(|_| QueueError::WalWriteFailed)?;
        clear_file(&mut inner.file)?;
        inner.record_count = 0;
        inner.current_size = HEADER_SIZE as u64;
        inner.last_sync = Instant::now();
        Ok(())
    }

    pub fn size(&self) -> Result<u64, QueueError> {
        let inner = self.inner.lock().map_err(|_| QueueError::WalWriteFailed)?;
        Ok(inner.current_size)
    }

    pub fn record_count(&self) -> Result<u64, QueueError> {
        let inner = self.inner.lock().map_err(|_| QueueError::WalWriteFailed)?;
        Ok(inner.record_count)
    }

    pub fn close(&self) -> Result<(), QueueError> {
        let inner = self.inner.lock().map_err(|_| QueueError::WalWriteFailed)?;
        inner.file.sync_all()?;
        Ok(())
    }

    pub fn path(&self) -> PathBuf {
        let inner = self.inner.lock().map_err(|_| QueueError::WalWriteFailed);
        match inner {
            Ok(inner) => inner.path.clone(),
            Err(_) => PathBuf::new(),
        }
    }
}

fn encode_item<T>(item: &T) -> Result<Vec<u8>, QueueError>
where
    T: Serialize,
{
    let json = serde_json::to_vec(item)
        .map_err(|err| QueueError::SerializationFailed(err.to_string()))?;
    let mut encoder = snap::raw::Encoder::new();
    encoder
        .compress_vec(&json)
        .map_err(|err| QueueError::SerializationFailed(err.to_string()))
}

fn decode_item<T>(data: &[u8]) -> Result<T, QueueError>
where
    T: DeserializeOwned,
{
    let mut decoder = snap::raw::Decoder::new();
    let decompressed = decoder
        .decompress_vec(data)
        .map_err(|err| QueueError::DeserializationFailed(err.to_string()))?;
    serde_json::from_slice(&decompressed)
        .map_err(|err| QueueError::DeserializationFailed(err.to_string()))
}

fn build_record(record_id: u64, payload: Vec<u8>) -> Result<Vec<u8>, QueueError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as i64;
    let data_len = payload.len() as u32;
    let checksum = crc32fast::hash(&payload);

    let mut buffer = Vec::with_capacity(8 + 8 + 4 + payload.len() + 4);
    buffer.extend_from_slice(&record_id.to_le_bytes());
    buffer.extend_from_slice(&timestamp.to_le_bytes());
    buffer.extend_from_slice(&data_len.to_le_bytes());
    buffer.extend_from_slice(&payload);
    buffer.extend_from_slice(&checksum.to_le_bytes());
    Ok(buffer)
}

fn read_records<T>(file: &File) -> Result<Vec<T>, QueueError>
where
    T: DeserializeOwned,
{
    let mut reader = file.try_clone()?;
    reader.seek(SeekFrom::Start(0))?;

    let mut header_bytes = [0u8; HEADER_SIZE];
    reader.read_exact(&mut header_bytes)?;
    parse_header(&header_bytes)?;

    let mut items = Vec::new();
    loop {
        let mut record_id_buf = [0u8; 8];
        if reader.read_exact(&mut record_id_buf).is_err() {
            break;
        }
        let mut timestamp_buf = [0u8; 8];
        reader.read_exact(&mut timestamp_buf)?;
        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf)?;

        let data_len = u32::from_le_bytes(len_buf) as usize;
        let mut data = vec![0u8; data_len];
        reader.read_exact(&mut data)?;

        let mut checksum_buf = [0u8; 4];
        reader.read_exact(&mut checksum_buf)?;
        let checksum = u32::from_le_bytes(checksum_buf);
        if crc32fast::hash(&data) != checksum {
            return Err(QueueError::ChecksumMismatch);
        }

        let item = decode_item(&data)?;
        items.push(item);
    }

    Ok(items)
}

fn write_header(file: &mut File, record_count: u64) -> Result<(), QueueError> {
    let mut bytes = [0u8; HEADER_SIZE];
    bytes[0..4].copy_from_slice(&WAL_MAGIC.to_le_bytes());
    bytes[4..6].copy_from_slice(&WAL_VERSION.to_le_bytes());
    bytes[6] = WAL_COMPRESSION_SNAPPY;
    bytes[7] = 0;
    bytes[8..16].copy_from_slice(&record_count.to_le_bytes());
    let checksum = crc32fast::hash(&bytes[0..16]);
    bytes[16..20].copy_from_slice(&checksum.to_le_bytes());

    file.seek(SeekFrom::Start(0))?;
    file.write_all(&bytes)?;
    file.flush()?;
    Ok(())
}

fn parse_header(bytes: &[u8; HEADER_SIZE]) -> Result<u64, QueueError> {
    let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    if magic != WAL_MAGIC {
        return Err(QueueError::DeserializationFailed("invalid wal magic".into()));
    }
    let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
    if version != WAL_VERSION {
        return Err(QueueError::DeserializationFailed("unsupported wal version".into()));
    }
    let compression = bytes[6];
    if compression != WAL_COMPRESSION_SNAPPY {
        return Err(QueueError::DeserializationFailed("unsupported wal compression".into()));
    }
    let checksum = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    if crc32fast::hash(&bytes[0..16]) != checksum {
        return Err(QueueError::ChecksumMismatch);
    }
    Ok(u64::from_le_bytes(bytes[8..16].try_into().unwrap()))
}

fn clear_file(file: &mut File) -> Result<(), QueueError> {
    file.set_len(0)?;
    write_header(file, 0)?;
    Ok(())
}

fn maybe_sync(inner: &mut WalInner) -> Result<(), QueueError> {
    match inner.sync_mode {
        WalSyncMode::Immediate => {
            inner.file.sync_data()?;
            inner.last_sync = Instant::now();
        }
        WalSyncMode::Periodic => {
            if inner.last_sync.elapsed() >= inner.sync_interval {
                inner.file.sync_data()?;
                inner.last_sync = Instant::now();
            }
        }
    }
    Ok(())
}
