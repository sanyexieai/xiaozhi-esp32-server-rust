use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::Value;
use xiaozhi_asr::{create_asr, AsrProvider};
use xiaozhi_config::ResourcePoolConfig;
use xiaozhi_core::Result;
use xiaozhi_llm::{create_llm, LlmProvider};
use xiaozhi_pool::{register_collector, PoolStatsCollector, PooledHandle, ResourcePool};
use xiaozhi_tts::{create_tts, TtsProvider};
use xiaozhi_vad::{create_vad, VadProvider};

fn config_with_provider(provider: &str, config: &Value) -> Value {
    let mut cfg = config.clone();
    if let Some(obj) = cfg.as_object_mut() {
        obj.entry("provider".to_string())
            .or_insert_with(|| Value::String(provider.to_string()));
    }
    cfg
}

fn provider_from_config(config: &Value) -> Option<&str> {
    config
        .get("provider")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

#[derive(Clone)]
pub struct PooledAsr(Arc<dyn AsrProvider>);

impl PooledAsr {
    pub fn arc(&self) -> Arc<dyn AsrProvider> {
        Arc::clone(&self.0)
    }
}

#[derive(Clone)]
pub struct PooledTts(Arc<dyn TtsProvider>);

impl PooledTts {
    pub fn arc(&self) -> Arc<dyn TtsProvider> {
        Arc::clone(&self.0)
    }
}

#[derive(Clone)]
pub struct PooledLlm(Arc<dyn LlmProvider>);

impl PooledLlm {
    pub fn arc(&self) -> Arc<dyn LlmProvider> {
        Arc::clone(&self.0)
    }
}

#[derive(Clone)]
pub struct PooledVad(Arc<Mutex<Box<dyn VadProvider>>>);

impl PooledVad {
    pub fn into_provider(self) -> Box<dyn VadProvider> {
        Box::new(PooledVadAdapter(Arc::clone(&self.0)))
    }
}

struct PooledVadAdapter(Arc<Mutex<Box<dyn VadProvider>>>);

impl VadProvider for PooledVadAdapter {
    fn is_vad(&mut self, pcm: &[i16]) -> Result<bool> {
        self.0.lock().is_vad(pcm)
    }

    fn reset(&mut self) {
        self.0.lock().reset();
    }

    fn close(&mut self) -> Result<()> {
        self.0.lock().close()
    }

    fn is_valid(&self) -> bool {
        self.0.lock().is_valid()
    }
}

pub struct SharedResourcePools {
    vad: ResourcePool<PooledVad>,
    asr: ResourcePool<PooledAsr>,
    tts: ResourcePool<PooledTts>,
    llm: ResourcePool<PooledLlm>,
}

impl SharedResourcePools {
    pub fn new(config: ResourcePoolConfig) -> Arc<Self> {
        let vad_factory = {
            Arc::new(move |_pool_key: &str, cfg: &Value| -> Result<Arc<PooledVad>> {
                let provider = provider_from_config(cfg).unwrap_or(xiaozhi_core::vad::WEBRTC);
                let vad = create_vad(provider, cfg)?;
                Ok(Arc::new(PooledVad(Arc::new(Mutex::new(vad)))))
            })
        };
        let asr_factory = {
            Arc::new(move |_pool_key: &str, cfg: &Value| -> Result<Arc<PooledAsr>> {
                let provider = provider_from_config(cfg).unwrap_or(xiaozhi_core::asr::FUNASR);
                create_asr(provider, cfg).map(|p| Arc::new(PooledAsr(p)))
            })
        };
        let tts_factory = {
            Arc::new(move |_pool_key: &str, cfg: &Value| -> Result<Arc<PooledTts>> {
                let provider = provider_from_config(cfg).unwrap_or(xiaozhi_core::tts::EDGE);
                create_tts(provider, cfg).map(|p| Arc::new(PooledTts(p)))
            })
        };
        let llm_factory = {
            Arc::new(move |_pool_key: &str, cfg: &Value| -> Result<Arc<PooledLlm>> {
                let provider = provider_from_config(cfg).unwrap_or(xiaozhi_core::llm::OPENAI);
                create_llm(provider, cfg).map(|p| Arc::new(PooledLlm(p)))
            })
        };

        Arc::new(Self {
            vad: ResourcePool::new(config.clone(), vad_factory),
            asr: ResourcePool::new(config.clone(), asr_factory),
            tts: ResourcePool::new(config.clone(), tts_factory),
            llm: ResourcePool::new(config, llm_factory),
        })
    }

    pub fn acquire_vad(
        &self,
        provider: &str,
        config: &Value,
    ) -> Result<(PooledHandle<PooledVad>, Box<dyn VadProvider>)> {
        let config = config_with_provider(provider, config);
        let key = xiaozhi_pool::generate_config_key(provider, &config);
        let handle = self.vad.acquire(&format!("vad:{key}"), &config)?;
        let mut vad = handle.get().clone().into_provider();
        vad.reset();
        Ok((handle, vad))
    }

    pub fn acquire_asr(
        &self,
        provider: &str,
        config: &Value,
    ) -> Result<PooledHandle<PooledAsr>> {
        let config = config_with_provider(provider, config);
        let key = xiaozhi_pool::generate_config_key(provider, &config);
        self.asr.acquire(&format!("asr:{key}"), &config)
    }

    pub fn acquire_tts(
        &self,
        provider: &str,
        config: &Value,
    ) -> Result<PooledHandle<PooledTts>> {
        let config = config_with_provider(provider, config);
        let key = xiaozhi_pool::generate_config_key(provider, &config);
        self.tts.acquire(&format!("tts:{key}"), &config)
    }

    pub fn acquire_llm(
        &self,
        provider: &str,
        config: &Value,
    ) -> Result<PooledHandle<PooledLlm>> {
        let config = config_with_provider(provider, config);
        let key = xiaozhi_pool::generate_config_key(provider, &config);
        self.llm.acquire(&format!("llm:{key}"), &config)
    }
}

pub struct SessionPoolHandles {
    pub _vad: PooledHandle<PooledVad>,
    pub _asr: PooledHandle<PooledAsr>,
    pub _tts: PooledHandle<PooledTts>,
    pub _llm: PooledHandle<PooledLlm>,
}

impl SharedResourcePools {
    pub fn acquire_session_resources(
        &self,
        vad_provider: &str,
        vad_config: &Value,
        asr_provider: &str,
        asr_config: &Value,
        llm_provider: &str,
        llm_config: &Value,
        tts_provider: &str,
        tts_config: &Value,
    ) -> Result<(
        Box<dyn VadProvider>,
        Arc<dyn AsrProvider>,
        Arc<dyn LlmProvider>,
        Arc<dyn TtsProvider>,
        SessionPoolHandles,
    )> {
        let (vad_handle, vad) = self.acquire_vad(vad_provider, vad_config)?;
        let asr_handle = self.acquire_asr(asr_provider, asr_config)?;
        let llm_handle = self.acquire_llm(llm_provider, llm_config)?;
        let tts_handle = self.acquire_tts(tts_provider, tts_config)?;
        let asr = asr_handle.get().arc();
        let llm = llm_handle.get().arc();
        let tts = tts_handle.get().arc();
        let handles = SessionPoolHandles {
            _vad: vad_handle,
            _asr: asr_handle,
            _tts: tts_handle,
            _llm: llm_handle,
        };
        Ok((vad, asr, llm, tts, handles))
    }

    pub fn register_stats_collectors(self: &Arc<Self>) {
        let vad_pool = Arc::clone(self);
        register_collector(Arc::new(VadPoolCollector {
            pools: vad_pool,
        }));
        let asr_pool = Arc::clone(self);
        register_collector(Arc::new(AsrPoolCollector {
            pools: asr_pool,
        }));
        let tts_pool = Arc::clone(self);
        register_collector(Arc::new(TtsPoolCollector {
            pools: tts_pool,
        }));
        let llm_pool = Arc::clone(self);
        register_collector(Arc::new(LlmPoolCollector {
            pools: llm_pool,
        }));
    }
}

struct VadPoolCollector {
    pools: Arc<SharedResourcePools>,
}

impl PoolStatsCollector for VadPoolCollector {
    fn collect(&self) -> HashMap<String, Value> {
        self.pools
            .vad
            .runtime_snapshot()
            .into_iter()
            .map(|(k, v)| (k, v))
            .collect()
    }
}

struct AsrPoolCollector {
    pools: Arc<SharedResourcePools>,
}

impl PoolStatsCollector for AsrPoolCollector {
    fn collect(&self) -> HashMap<String, Value> {
        self.pools
            .asr
            .runtime_snapshot()
            .into_iter()
            .map(|(k, v)| (k, v))
            .collect()
    }
}

struct TtsPoolCollector {
    pools: Arc<SharedResourcePools>,
}

impl PoolStatsCollector for TtsPoolCollector {
    fn collect(&self) -> HashMap<String, Value> {
        self.pools
            .tts
            .runtime_snapshot()
            .into_iter()
            .map(|(k, v)| (k, v))
            .collect()
    }
}

struct LlmPoolCollector {
    pools: Arc<SharedResourcePools>,
}

impl PoolStatsCollector for LlmPoolCollector {
    fn collect(&self) -> HashMap<String, Value> {
        self.pools
            .llm
            .runtime_snapshot()
            .into_iter()
            .map(|(k, v)| (k, v))
            .collect()
    }
}
