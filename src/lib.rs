include!("app.rs");

/// SDL's Android Activity looks up this symbol in `libmain.so` and invokes it
/// on the native SDL thread.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn SDL_main(_argc: i32, _argv: *mut *mut std::ffi::c_char) -> i32 {
    let arguments = if _argc > 1 && !_argv.is_null() {
        unsafe {
            std::slice::from_raw_parts(_argv, _argc as usize)
                .iter()
                .skip(1)
                .filter_map(|argument| {
                    (!argument.is_null()).then(|| {
                        std::ffi::CStr::from_ptr(*argument)
                            .to_string_lossy()
                            .into_owned()
                    })
                })
                .collect::<Vec<_>>()
        }
    } else {
        Vec::new()
    };
    match run_with_args(arguments.into_iter()) {
        Ok(()) => 0,
        Err(error) => {
            let message = format!("Ikura failed: {error}");
            sdl2::log::log(&message);
            let _ = sdl2::messagebox::show_simple_message_box(
                sdl2::messagebox::MessageBoxFlag::ERROR,
                "游戏启动失败",
                &message,
                None,
            );
            1
        }
    }
}
