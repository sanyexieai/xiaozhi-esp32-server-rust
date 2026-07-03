//! Provider 名称常量，与 Go 版 constants 包对齐

pub mod vad {
    pub const SILERO: &str = "silero_vad";
    pub const WEBRTC: &str = "webrtc_vad";
    pub const TEN: &str = "ten_vad";
}

pub mod asr {
    pub const FUNASR: &str = "funasr";
    pub const DOUBAO: &str = "doubao";
    pub const ALIYUN_FUNASR: &str = "aliyun_funasr";
    pub const ALIYUN_QWEN3: &str = "aliyun_qwen3";
    pub const XUNFEI: &str = "xunfei";
}

pub mod llm {
    pub const OPENAI: &str = "openai";
    pub const OLLAMA: &str = "ollama";
    pub const EINO: &str = "eino";
    pub const EINO_LLM: &str = "eino_llm";
    pub const DIFY: &str = "dify";
    pub const COZE: &str = "coze";
}

pub mod tts {
    pub const DOUBAO: &str = "doubao";
    pub const DOUBAO_WS: &str = "doubao_ws";
    pub const COSYVOICE: &str = "cosyvoice";
    pub const EDGE: &str = "edge";
    pub const EDGE_OFFLINE: &str = "edge_offline";
    pub const XIAOZHI: &str = "xiaozhi";
    pub const XUNFEI: &str = "xunfei";
    pub const XUNFEI_SUPER: &str = "xunfei_super_tts";
    pub const OPENAI: &str = "openai";
    pub const ZHIPU: &str = "zhipu";
    pub const MINIMAX: &str = "minimax";
    pub const ALIYUN_QWEN: &str = "aliyun_qwen";
    pub const INDEXTTS_VLLM: &str = "indextts_vllm";
}

pub mod memory {
    pub const NOMEMO: &str = "nomemo";
    pub const MEMOBASE: &str = "memobase";
    pub const MEM0: &str = "mem0";
    pub const MEMOS: &str = "memos";
}

pub mod transport {
    pub const WEBSOCKET: &str = "websocket";
    pub const MQTT_UDP: &str = "udp";
}

pub mod message {
    pub const HELLO: &str = "hello";
    pub const ABORT: &str = "abort";
    pub const LISTEN: &str = "listen";
    pub const IOT: &str = "iot";
    pub const MCP: &str = "mcp";
    pub const GOODBYE: &str = "goodbye";
    pub const SPEAK_READY: &str = "speak_ready";

    pub const STT: &str = "stt";
    pub const TTS: &str = "tts";
    pub const LLM: &str = "llm";
    pub const TEXT: &str = "text";
    pub const SPEAK_REQUEST: &str = "speak_request";

    pub const START: &str = "start";
    pub const STOP: &str = "stop";
    pub const DETECT: &str = "detect";
    pub const SUCCESS: &str = "success";
    pub const READY: &str = "ready";
    pub const SENTENCE_START: &str = "sentence_start";
    pub const SENTENCE_END: &str = "sentence_end";
}

pub mod rag {
    pub const DIFY: &str = "dify";
    pub const RAGFLOW: &str = "ragflow";
    pub const WEKNORA: &str = "weknora";
    pub const LOCAL: &str = "local";
}

/// 管理后台 OTA 连通性测试使用的虚拟设备，不应写入 devices 表
pub mod ota_test {
    pub const DEVICE_ID: &str = "ota-test-device";
    pub const CLIENT_ID: &str = "ota-test-client";

    pub fn is_probe_device(device_id: &str) -> bool {
        device_id.eq_ignore_ascii_case(DEVICE_ID)
    }
}

/// 管理台设备对话模拟器：独立 WebSocket 会话，避免与真实设备抢占 ChatManager
pub mod simulator {
    pub const DEVICE_ID_PREFIX: &str = "sim:";

    pub fn wrap_device_id(physical_device_id: &str) -> String {
        let physical = physical_device_id.trim();
        if physical.is_empty() {
            return String::new();
        }
        if is_simulator_device(physical) {
            return physical.to_string();
        }
        format!("{DEVICE_ID_PREFIX}{physical}")
    }

    pub fn is_simulator_device(device_id: &str) -> bool {
        device_id.starts_with(DEVICE_ID_PREFIX) || device_id.starts_with("web-sim-")
    }

    /// OTA 探针、Web 模拟设备等不应出现在设备管理列表
    pub fn is_hidden_list_device(device_id: &str) -> bool {
        super::ota_test::is_probe_device(device_id) || is_simulator_device(device_id)
    }

    pub fn resolve_physical_device_id(device_id: &str) -> &str {
        device_id
            .strip_prefix(DEVICE_ID_PREFIX)
            .unwrap_or(device_id)
            .trim()
    }

    pub fn web_sim_device_id(user_id: i64) -> String {
        format!("web-sim-{user_id}")
    }
}

pub mod config_provider {
    pub const MANAGER: &str = "manager";
    pub const REDIS: &str = "redis";
}
