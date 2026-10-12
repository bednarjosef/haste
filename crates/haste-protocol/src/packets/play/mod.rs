//! Play state.
pub mod clientbound;
pub mod serverbound;
pub use clientbound::ClientboundPlay;
pub use serverbound::ServerboundPlay;

#[cfg(test)]
mod tests {
    use crate::packets::{Direction, PacketIds, State};

    fn missing(ids: &PacketIds, dir: Direction, names: &[&str]) -> Vec<String> {
        ids.iter(State::Play, dir)
            .filter(|(_, n)| !names.contains(n))
            .map(|(_, n)| n.to_string())
            .collect()
    }

    #[test]
    #[ignore = "needs cache/datagen/26.3/reports/packets.json (run with --ignored)"]
    fn every_play_packet_has_a_struct() {
        let ids = PacketIds::load().expect("packets.json");
        let cb: Vec<&str> = super::ClientboundPlay::DECODERS
            .iter()
            .map(|(n, _)| *n)
            .collect();
        let sb: Vec<&str> = super::ServerboundPlay::DECODERS
            .iter()
            .map(|(n, _)| *n)
            .collect();
        assert_eq!(
            missing(&ids, Direction::Clientbound, &cb),
            Vec::<String>::new()
        );
        assert_eq!(
            missing(&ids, Direction::Serverbound, &sb),
            Vec::<String>::new()
        );
        assert_eq!(cb.len(), ids.len(State::Play, Direction::Clientbound));
        assert_eq!(sb.len(), ids.len(State::Play, Direction::Serverbound));
    }
}
