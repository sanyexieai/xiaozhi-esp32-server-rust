use std::ptr;

use xiaozhi_core::{Error, Result};

use crate::ffi;
use crate::traits::VadProvider;

pub struct TenVad {
    handle: ffi::ten_vad_handle_t,
    hop_size: usize,
    pcm_buffer: Vec<i16>,
}

// C 句柄由资源池 Mutex 串行访问，与 Go 版 ten_vad 一致。
unsafe impl Send for TenVad {}
unsafe impl Sync for TenVad {}

impl TenVad {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let hop_size = config
            .get("hop_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(512) as usize;
        let threshold = config
            .get("threshold")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.3) as f32;

        let mut handle: ffi::ten_vad_handle_t = ptr::null_mut();
        let ret = unsafe { ffi::ten_vad_create(&mut handle, hop_size, threshold) };
        if ret != 0 || handle.is_null() {
            return Err(Error::Audio(format!(
                "ten_vad_create 失败: ret={ret}, hop_size={hop_size}, threshold={threshold}"
            )));
        }

        let version_ptr = unsafe { ffi::ten_vad_get_version() };
        if !version_ptr.is_null() {
            let version = unsafe { std::ffi::CStr::from_ptr(version_ptr) };
            tracing::info!(
                hop_size,
                threshold,
                version = %version.to_string_lossy(),
                "TEN-VAD 实例创建成功"
            );
        } else {
            tracing::info!(hop_size, threshold, "TEN-VAD 实例创建成功");
        }

        Ok(Self {
            handle,
            hop_size,
            pcm_buffer: Vec::new(),
        })
    }

    fn process_hop(&mut self, frame: &[i16]) -> Result<bool> {
        debug_assert_eq!(frame.len(), self.hop_size);
        let mut prob: f32 = 0.0;
        let mut flag: i32 = 0;
        let ret = unsafe {
            ffi::ten_vad_process(
                self.handle,
                frame.as_ptr(),
                frame.len(),
                &mut prob,
                &mut flag,
            )
        };
        if ret != 0 {
            return Err(Error::Audio("ten_vad_process 失败".into()));
        }
        Ok(flag == 1)
    }
}

impl VadProvider for TenVad {
    fn is_vad(&mut self, pcm: &[i16]) -> Result<bool> {
        if pcm.is_empty() {
            return Ok(false);
        }

        self.pcm_buffer.extend_from_slice(pcm);
        let mut voice = false;
        while self.pcm_buffer.len() >= self.hop_size {
            let frame: Vec<i16> = self.pcm_buffer.drain(..self.hop_size).collect();
            if self.process_hop(&frame)? {
                voice = true;
            }
        }
        Ok(voice)
    }

    fn reset(&mut self) {
        self.pcm_buffer.clear();
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn is_valid(&self) -> bool {
        !self.handle.is_null()
    }
}

impl Drop for TenVad {
    fn drop(&mut self) {
        if self.handle.is_null() {
            return;
        }
        let mut handle = self.handle;
        let ret = unsafe { ffi::ten_vad_destroy(&mut handle) };
        if ret != 0 {
            tracing::warn!("ten_vad_destroy 失败: ret={ret}");
        }
        self.handle = ptr::null_mut();
    }
}
