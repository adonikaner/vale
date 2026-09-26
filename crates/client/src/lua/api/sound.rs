//! **The interface's four sound verbs** — `PlaySound`, `PlaySoundFile`,
//! `PlayMusic`, `StopMusic` — which record, like every write.
//!
//! The same shape as [`super::super::panels::worldmap`] and for the same reason: a handler
//! cannot reach the mixer directly, because the world is borrowed for the
//! length of the call — so the verb pushes a request and
//! `crate::sound::interface` drains and plays. What a *name* means
//! (`PlaySound("igMainMenuOption")` is a lookup into `SoundEntries.dbc`'s name
//! column) is decided there, where the bank is, not here.

use std::cell::RefCell;
use std::rc::Rc;

/// A play the interface asked for — see the module comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoundRequest {
    /// `PlaySound("igMainMenuOption")` — a `SoundEntries` **name**.
    Named(String),
    /// `PlaySoundFile("Sound\\...")` — an archive path, played as it is.
    File(String),
    /// `PlayMusic("Sound\\...")` — an archive path, looped on the music
    /// channel until [`SoundRequest::StopMusic`] or a zone takes it back.
    Music(String),
    StopMusic,
}

pub type SoundQueue = Rc<RefCell<Vec<SoundRequest>>>;

/// Register the four verbs. Unscoped — they record.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &SoundQueue) -> mlua::Result<()> {
    let globals = lua.globals();

    macro_rules! push {
        ($name:expr, $args:ty, |$arg:ident| $body:expr) => {{
            let queue = Rc::clone(queue);
            let f = lua.create_function(move |_, $arg: $args| {
                if let Some(request) = $body {
                    queue.borrow_mut().push(request);
                }
                Ok(())
            })?;
            globals.set($name, f)?;
        }};
    }

    // A nil argument records nothing rather than erroring: 5875's own C side
    // tolerates it, and one panel passing nil would otherwise kill its whole
    // handler for a missing click noise.
    push!("PlaySound", Option<String>, |name| name
        .map(SoundRequest::Named));
    push!("PlaySoundFile", Option<String>, |path| path
        .map(SoundRequest::File));
    push!("PlayMusic", Option<String>, |path| path
        .map(SoundRequest::Music));
    push!("StopMusic", (), |_a| Some(SoundRequest::StopMusic));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four verbs record in call order, answer nothing (a verb has no
    /// return), and tolerate the nil argument 5875's C side tolerates.
    #[test]
    fn the_four_verbs_record_and_answer_nothing() {
        let lua = mlua::Lua::new();
        let queue: SoundQueue = Rc::new(RefCell::new(Vec::new()));
        register(&lua, &queue).expect("registers");

        let value: mlua::Value = lua
            .load(
                r#"PlaySound("igMainMenuOption");
                   PlaySoundFile("Sound\\x.wav");
                   PlayMusic("Sound\\m.mp3");
                   StopMusic();
                   PlaySound(nil);
                   return PlaySound("LEVELUPSOUND")"#,
            )
            .eval()
            .expect("the chunk runs");
        assert!(value.is_nil(), "a verb answers nothing");
        assert_eq!(
            *queue.borrow(),
            vec![
                SoundRequest::Named("igMainMenuOption".into()),
                SoundRequest::File("Sound\\x.wav".into()),
                SoundRequest::Music("Sound\\m.mp3".into()),
                SoundRequest::StopMusic,
                SoundRequest::Named("LEVELUPSOUND".into()),
            ],
            "in call order, with the nil recorded as nothing"
        );
    }
}
