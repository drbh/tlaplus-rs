use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use std::{num::NonZeroUsize, path::Path};

pub type Error = Box<dyn std::error::Error + Send + Sync>;
const KEYS: TableDefinition<&[u8], u64> = TableDefinition::new("keys");
const NODES: TableDefinition<u64, &[u8]> = TableDefinition::new("nodes");
const META: TableDefinition<&str, u64> = TableDefinition::new("metadata");

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Node {
    pub state: Vec<u8>,
    pub parent: Option<(u64, u64)>,
    pub depth: u64,
}

impl Node {
    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(24 + self.state.len());
        let (parent, action) = self.parent.unwrap_or((u64::MAX, 0));
        for n in [self.depth, parent, action] {
            bytes.extend(n.to_le_bytes());
        }
        bytes.extend(&self.state);
        bytes
    }
    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 24 {
            return Err("truncated disk node".into());
        }
        let mut words = bytes[..24]
            .chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()));
        let depth = words.next().unwrap();
        let parent = words.next().unwrap();
        let action = words.next().unwrap();
        Ok(Self {
            depth,
            parent: (parent != u64::MAX).then_some((parent, action)),
            state: bytes[24..].to_vec(),
        })
    }
}

pub struct Store {
    db: Database,
}

impl Store {
    /// The budget bounds the database page cache, not evaluator or caller allocations.
    pub fn open(path: impl AsRef<Path>, cache_bytes: NonZeroUsize) -> Result<Self, Error> {
        let db = Database::builder()
            .set_cache_size(cache_bytes.get())
            .create(path)?;
        let tx = db.begin_write()?;
        {
            tx.open_table(KEYS)?;
            tx.open_table(NODES)?;
            let mut meta = tx.open_table(META)?;
            for key in ["count", "head"] {
                if meta.get(key)?.is_none() {
                    meta.insert(key, 0)?;
                }
            }
        }
        tx.commit()?;
        Ok(Self { db })
    }
    pub fn len(&self) -> Result<u64, Error> {
        self.metadata("count")
    }
    pub fn is_empty(&self) -> Result<bool, Error> {
        Ok(self.len()? == 0)
    }
    pub fn queued(&self) -> Result<u64, Error> {
        Ok(self.len()? - self.metadata("head")?)
    }
    fn metadata(&self, key: &str) -> Result<u64, Error> {
        Ok(self
            .db
            .begin_read()?
            .open_table(META)?
            .get(key)?
            .ok_or("missing store metadata")?
            .value())
    }
    pub fn find(&self, key: &[u8]) -> Result<Option<u64>, Error> {
        Ok(self
            .db
            .begin_read()?
            .open_table(KEYS)?
            .get(key)?
            .map(|v| v.value()))
    }
    pub fn get(&self, id: u64) -> Result<Option<Node>, Error> {
        self.db
            .begin_read()?
            .open_table(NODES)?
            .get(id)?
            .map(|v| Node::decode(v.value()))
            .transpose()
    }
    pub fn insert(&self, key: &[u8], node: &Node) -> Result<(u64, bool), Error> {
        let mut tx = self.db.begin_write()?;
        tx.set_durability(redb::Durability::None)?;
        let result;
        {
            let mut keys = tx.open_table(KEYS)?;
            if let Some(id) = keys.get(key)? {
                return Ok((id.value(), false));
            }
            let mut nodes = tx.open_table(NODES)?;
            let mut meta = tx.open_table(META)?;
            let count = meta.get("count")?.ok_or("missing count")?.value();
            if count == u64::MAX {
                return Err("disk node identifier overflow".into());
            }
            if let Some((parent, _)) = node.parent {
                if parent >= count {
                    return Err("parent must precede its child".into());
                }
                let parent = nodes.get(parent)?.ok_or("missing parent")?;
                if Node::decode(parent.value())?.depth.checked_add(1) != Some(node.depth) {
                    return Err("invalid child depth".into());
                }
            } else if node.depth != 1 {
                return Err("initial depth must be one".into());
            }
            keys.insert(key, count)?;
            nodes.insert(count, node.encode().as_slice())?;
            meta.insert("count", count + 1)?;
            result = (count, true);
        }
        tx.commit()?;
        Ok(result)
    }
    /// Queue order is insertion order; records remain available for trace replay.
    pub fn pop(&self) -> Result<Option<(u64, Node)>, Error> {
        let mut tx = self.db.begin_write()?;
        tx.set_durability(redb::Durability::None)?;
        let result;
        {
            let mut meta = tx.open_table(META)?;
            let head = meta.get("head")?.ok_or("missing head")?.value();
            let count = meta.get("count")?.ok_or("missing count")?.value();
            if head == count {
                return Ok(None);
            }
            let nodes = tx.open_table(NODES)?;
            let node = nodes.get(head)?.ok_or("missing queued state")?;
            result = (head, Node::decode(node.value())?);
            meta.insert("head", head + 1)?;
        }
        tx.commit()?;
        Ok(Some(result))
    }
    pub fn trace(&self, mut id: u64) -> Result<Vec<(u64, Node)>, Error> {
        let mut result = Vec::new();
        loop {
            let node = self.get(id)?.ok_or("trace refers to a missing state")?;
            let parent = node.parent;
            result.push((id, node));
            match parent {
                Some((previous, _)) => id = previous,
                None => break,
            }
        }
        result.reverse();
        Ok(result)
    }
}
