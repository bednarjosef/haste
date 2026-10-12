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
            NativeMethod::from_raw_parts(jni_str!("textWidth"), jni_str!("(Ljava/lang/String;)I"), text::text_width as *mut _),
            NativeMethod::from_raw_parts(jni_str!("charWidth"), jni_str!("(I)I"), text::char_width as *mut _),
            NativeMethod::from_raw_parts(jni_str!("lineHeight"), jni_str!("()I"), text::line_height as *mut _),
            NativeMethod::from_raw_parts(jni_str!("stackItem"), jni_str!("(J)I"), state::stack_item as *mut _),
            NativeMethod::from_raw_parts(jni_str!("stackCount"), jni_str!("(J)I"), state::stack_count as *mut _),
            NativeMethod::from_raw_parts(jni_str!("stackSetCount"), jni_str!("(JI)V"), state::stack_set_count as *mut _),
            NativeMethod::from_raw_parts(jni_str!("itemName"), jni_str!("(I)Ljava/lang/String;"), state::item_name as *mut _),
            NativeMethod::from_raw_parts(jni_str!("inventorySize"), jni_str!("()I"), state::inventory_size as *mut _),
            NativeMethod::from_raw_parts(jni_str!("selectedSlot"), jni_str!("()I"), state::selected_slot as *mut _),
            NativeMethod::from_raw_parts(jni_str!("chatLines"), jni_str!("()[Ljava/lang/String;"), state::chat_lines as *mut _),
            NativeMethod::from_raw_parts(jni_str!("sendChat"), jni_str!("(Ljava/lang/String;Z)V"), state::send_chat as *mut _),
            NativeMethod::from_raw_parts(jni_str!("playerName"), jni_str!("()Ljava/lang/String;"), state::player_name as *mut _),
        ]
    };
    // Safety: see above.
    unsafe { env.register_native_methods(&cls, &methods)? };
    Ok(())
}
