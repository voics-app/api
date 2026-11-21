//! IDs reserved at the Voics epoch for the built-in sticker pack.
use domain::sticker::Sticker;
use domain::Snowflake;
use serde_json::{json, Value};

const ITEMS: [(u64, &str, &str, [f64; 3]); 3] = [
    (1, "Happy", "😀", [1.0, 0.78, 0.28]),
    (2, "Love", "❤️", [1.0, 0.49, 0.58]),
    (3, "Celebrate", "🎉", [0.56, 0.66, 1.0]),
];

pub fn all() -> Vec<Sticker> {
    ITEMS
        .iter()
        .map(|(id, name, tags, _)| Sticker {
            id: Snowflake::new(*id),
            guild_id: Snowflake::new(0),
            creator_id: None,
            name: (*name).to_owned(),
            tags: (*tags).to_owned(),
            format_type: 3,
            available: true,
        })
        .collect()
}

pub fn find(id: Snowflake) -> Option<Sticker> {
    all().into_iter().find(|sticker| sticker.id == id)
}

/// A self-contained Lottie animation, with no external resources or expressions.
pub fn asset(id: Snowflake) -> Option<Value> {
    let (_, name, _, color) = ITEMS.iter().find(|(item, ..)| *item == id.value())?;
    let transform = json!({"o":{"a":0,"k":100},"r":{"a":0,"k":0},"p":{"a":0,"k":[160,160,0]},"a":{"a":0,"k":[0,0,0]},"s":{"a":0,"k":[100,100,100]}});
    let group_transform = json!({"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100},"sk":{"a":0,"k":0},"sa":{"a":0,"k":0}});
    let face = json!({"ty":"gr","it":[{"ty":"el","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[246,246]}},{"ty":"fl","c":{"a":0,"k":[color[0],color[1],color[2],1]},"o":{"a":0,"k":100},"r":1},group_transform]});
    let eye = |x| json!({"ty":"gr","it":[{"ty":"el","p":{"a":0,"k":[x,-25]},"s":{"a":0,"k":[18,28]}},{"ty":"fl","c":{"a":0,"k":[0.13,0.14,0.2,1]},"o":{"a":0,"k":100},"r":1},group_transform]});
    let smile = json!({"ty":"gr","it":[{"ty":"sh","ks":{"a":0,"k":{"i":[[0,0],[-20,0],[0,0]],"o":[[0,0],[20,0],[0,0]],"v":[[-48,35],[0,60],[48,35]],"c":false}}},{"ty":"st","c":{"a":0,"k":[0.13,0.14,0.2,1]},"o":{"a":0,"k":100},"w":{"a":0,"k":10},"lc":2,"lj":2},group_transform]});
    Some(
        json!({"v":"5.7.0","fr":30,"ip":0,"op":60,"w":320,"h":320,"nm":name,"ddd":0,"assets":[],"layers":[{"ddd":0,"ind":1,"ty":4,"nm":"Face","sr":1,"ks":transform,"ao":0,"shapes":[eye(-45),eye(45),smile,face],"ip":0,"op":60,"st":0}]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::images::ImageUpload;
    use crate::support::stickers::{self, StickerFormat};

    #[test]
    fn every_builtin_is_self_contained_and_has_a_valid_asset() {
        for sticker in all() {
            let bytes = serde_json::to_vec(&asset(sticker.id).unwrap()).unwrap();
            assert_eq!(
                stickers::validate(&ImageUpload {
                    bytes,
                    content_type: "application/json".into()
                })
                .unwrap(),
                StickerFormat::Lottie
            );
        }
        assert!(asset(Snowflake::new(999)).is_none());
    }
}
