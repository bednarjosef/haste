use super::*;

#[test]
fn decodes_every_opcode() {
    let strings = vec![
        "hi".to_owned(),
        "minecraft:widget/button".to_owned(),
        "minecraft:textures/gui/x.png".to_owned(),
    ];
    let fb = |x: f32| x.to_bits() as i32;
    let b = vec![
        FILL,
        0,
        0,
        10,
        10,
        -1,
        GRADIENT,
        0,
        0,
        5,
        5,
        1,
        2,
        TEXT,
        3,
        4,
        0xFFFFFF,
        1,
        0,
        TEXTURE,
        2,
        1,
        2,
        3,
        4,
        fb(0.5),
        fb(1.0),
        fb(2.0),
        fb(3.0),
        256,
        128,
        -1,
        0,
        SPRITE,
        1,
        10,
        20,
        200,
        20,
        -1,
        3,
        ITEM,
        7,
        8,
        9,
        64,
        SCISSOR_PUSH,
        1,
        2,
        3,
        4,
        SCISSOR_POP,
        TRANSLATE_PUSH,
        fb(1.5),
        fb(-2.0),
        fb(0.0),
        TRANSLATE_POP,
        SPRITE_REGION,
        1,
        182,
        5,
        0,
        0,
        10,
        20,
        91,
        5,
        -1,
        0,
        STYLED_TEXT,
        1,
        2,
        0xFFFFFF,
        0,
        0,
        1 | 4,
        ENTITY,
        42,
        1,
        2,
        3,
        4,
        fb(30.0),
        fb(5.5),
        fb(6.5),
    ];
    let c = decode(&b, &strings).unwrap();
    assert_eq!(c.len(), 13);
    assert!(
        matches!(&c[10], GuiCommand::SpriteRegion { sprite, sprite_w: 182, w: 91, x: 10, .. } if sprite.path() == "widget/button")
    );
    assert!(
        matches!(&c[11], GuiCommand::StyledText { style, text, .. } if style.bold && style.underline && !style.italic && text == "hi")
    );
    assert_eq!(
        c[12],
        GuiCommand::Entity {
            entity: EntityId(42),
            x1: 1,
            y1: 2,
            x2: 3,
            y2: 4,
            scale: 30.0,
            mouse_x: 5.5,
            mouse_y: 6.5
        }
    );
    assert_eq!(
        c[0],
        GuiCommand::Fill {
            x1: 0,
            y1: 0,
            x2: 10,
            y2: 10,
            color: 0xFFFF_FFFF
        }
    );
    assert_eq!(
        c[2],
        GuiCommand::Text {
            x: 3,
            y: 4,
            text: "hi".into(),
            color: 0xFFFFFF,
            shadow: true
        }
    );
    assert!(
        matches!(&c[3], GuiCommand::Texture { texture, u, tex_w: 256, blend: GuiBlend::Normal, .. } if texture.path() == "textures/gui/x.png" && *u == 0.5)
    );
    assert!(
        matches!(&c[4], GuiCommand::Sprite { sprite, w: 200, blend: GuiBlend::Additive, .. } if sprite.path() == "widget/button")
    );
    assert_eq!(
        c[5],
        GuiCommand::Item {
            item: RegistryIndex(7),
            x: 8,
            y: 9,
            count: 64
        }
    );
    assert_eq!(
        c[8],
        GuiCommand::TranslatePush {
            dx: 1.5,
            dy: -2.0,
            dz: 0.0
        }
    );
    assert_eq!(c[9], GuiCommand::TranslatePop);
}

#[test]
fn rejects_malformed_frames() {
    assert!(decode(&[FILL, 1, 2], &[]).is_err());
    assert!(decode(&[TEXT, 0, 0, 0, 0, 5], &[]).is_err());
    assert!(decode(&[99], &[]).is_err());
}

#[test]
fn decodes_blend_codes() {
    assert_eq!(blend_of(0), GuiBlend::Normal);
    assert_eq!(blend_of(1), GuiBlend::Multiply);
    assert_eq!(blend_of(2), GuiBlend::Invert);
    assert_eq!(blend_of(3), GuiBlend::Additive);
}
