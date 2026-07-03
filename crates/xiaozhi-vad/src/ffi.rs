#![allow(non_camel_case_types)]

use std::ffi::c_void;

pub type ten_vad_handle_t = *mut c_void;

extern "C" {
    pub fn ten_vad_create(
        handle: *mut ten_vad_handle_t,
        hop_size: usize,
        threshold: f32,
    ) -> i32;

    pub fn ten_vad_process(
        handle: ten_vad_handle_t,
        audio_data: *const i16,
        audio_data_length: usize,
        out_probability: *mut f32,
        out_flag: *mut i32,
    ) -> i32;

    pub fn ten_vad_destroy(handle: *mut ten_vad_handle_t) -> i32;

    pub fn ten_vad_get_version() -> *const i8;
}
