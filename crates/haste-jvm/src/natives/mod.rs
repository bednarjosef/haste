//! Java -> Rust natives of `haste.rt.Native`, registered once at startup.
mod state;
mod text;

use jni::{jni_str, Env, NativeMethod};

pub(crate) use text::set_metrics;

use crate::error::Result;

pub(crate) fn register(env: &mut Env) -> Result<()> {
    let cls = env.find_class(jni_str!("haste/rt/Native"))?;
    // Safety: every fn pointer matches its JNI signature (static natives: env, class, args...).
    let methods = unsafe {
        [
            NativeMethod::from_raw_parts(
                jni_str!("textWidth"),
                jni_str!("(Ljava/lang/String;)I"),
                text::text_width as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("charWidth"),
                jni_str!("(I)I"),
                text::char_width as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("lineHeight"),
                jni_str!("()I"),
                text::line_height as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("selectedSlot"),
                jni_str!("()I"),
                state::selected_slot as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("sendChat"),
                jni_str!("(Ljava/lang/String;Z)V"),
                state::send_chat as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("gameMode"),
                jni_str!("()I"),
                state::game_mode as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("permissionLevel"),
                jni_str!("()I"),
                state::permission_level as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("playerName"),
                jni_str!("()Ljava/lang/String;"),
                state::player_name as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("inventorySlot"),
                jni_str!("(I[B)V"),
                state::inventory_slot as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("mirror"),
                jni_str!("([I[D)V"),
                state::mirror as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("debugText"),
                jni_str!("()Ljava/lang/String;"),
                state::debug_text as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("requestDisconnect"),
                jni_str!("()V"),
                state::request_disconnect as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("requestJoin"),
                jni_str!("(Ljava/lang/String;)V"),
                state::request_join as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("cancelJoin"),
                jni_str!("()V"),
                state::cancel_join as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("requestQuit"),
                jni_str!("()V"),
                state::request_quit as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("requestOpenWorld"),
                jni_str!("(Ljava/lang/String;)V"),
                state::request_open_world as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("requestCreateWorld"),
                jni_str!("(Ljava/lang/String;Ljava/lang/String;IIZZLjava/lang/String;ZLjava/lang/String;)V"),
                state::request_create_world as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("localWorldRunning"),
                jni_str!("()Z"),
                state::local_world_running as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("startPing"),
                jni_str!("(Ljava/lang/String;)V"),
                state::start_ping as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("takePingResults"),
                jni_str!("()[Ljava/lang/String;"),
                state::take_ping_results as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("sendPacket"),
                jni_str!("([B)V"),
                state::send_packet as *mut _,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("optionsChanged"),
                jni_str!("(Ljava/lang/String;Ljava/lang/String;)V"),
                state::options_changed as *mut _,
            ),
        ]
    };
    // Safety: see above.
    unsafe { env.register_native_methods(&cls, &methods)? };
    Ok(())
}
