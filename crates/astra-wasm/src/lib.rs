#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::slice;

    use astra_compiler_api::{API_VERSION_MAJOR, process_json};

    #[unsafe(no_mangle)]
    pub extern "C" fn astra_protocol_version() -> u32 {
        API_VERSION_MAJOR
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn astra_alloc(len: usize) -> *mut u8 {
        let mut buffer = Vec::<u8>::with_capacity(len);
        let pointer = buffer.as_mut_ptr();
        std::mem::forget(buffer);
        pointer
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn astra_free(pointer: *mut u8, len: usize) {
        if pointer.is_null() || len == 0 {
            return;
        }

        unsafe {
            drop(Vec::from_raw_parts(pointer, 0, len));
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn astra_compile(pointer: *const u8, len: usize) -> u64 {
        let input = if pointer.is_null() || len == 0 {
            String::new()
        } else {
            let bytes = unsafe { slice::from_raw_parts(pointer, len) };
            String::from_utf8_lossy(bytes).into_owned()
        };

        let mut output = process_json(&input).into_bytes();
        output.shrink_to_fit();
        let output_len = output.len();
        let output_pointer = output.as_mut_ptr();
        std::mem::forget(output);

        ((output_pointer as u64) << 32) | output_len as u64
    }
}
