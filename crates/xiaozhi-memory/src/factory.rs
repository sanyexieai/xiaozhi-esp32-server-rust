use std::sync::Arc;

use xiaozhi_core::{Error, Result, memory as mem_const};

use crate::traits::MemoryProvider;
use crate::{mem0, memobase, memos, nomemo};

pub fn create_memory(provider: &str, config: &serde_json::Value) -> Result<Arc<dyn MemoryProvider>> {
    let effective = if provider.is_empty() {
        mem_const::NOMEMO
    } else {
        provider
    };

    match effective {
        mem_const::NOMEMO => Ok(Arc::new(nomemo::NoMemoProvider)),
        mem_const::MEMOBASE => Ok(Arc::new(memobase::MemobaseProvider::from_config(config)?)),
        mem_const::MEM0 => Ok(Arc::new(mem0::Mem0Provider::from_config(config)?)),
        mem_const::MEMOS => Ok(Arc::new(memos::MemosProvider::from_config(config)?)),
        other => Err(Error::Unsupported(format!("不支持的记忆类型: {other}"))),
    }
}
