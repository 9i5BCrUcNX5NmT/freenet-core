use blake3::Hash;
use serde::{Serialize, Deserialize};
use serde_bytes::ByteBuf;

#[derive(Debug, Clone)]
struct Blake3Hash(Hash);

#[derive(Serialize, Deserialize)]
pub struct Blake3HashWrapper(ByteBuf);

impl From<Blake3Hash> for Blake3HashWrapper {
    fn from(hash: Blake3Hash) -> Self {
        Blake3HashWrapper(ByteBuf::from(hash.0.as_bytes().to_vec()))
    }
}

impl From<Blake3HashWrapper> for Blake3Hash {
    fn from(wrapper: Blake3HashWrapper) -> Self {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&wrapper.0[..]);
        Blake3Hash(Hash::from(arr))
    }
}
