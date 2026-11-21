//! Linking two folded ports channel by channel.

// Rust guideline compliant 2026-02-21

/// The port pairs to link from `outputs` to `inputs`.
///
/// Each side is a list of `(port id, channel)`, in port order. A mono side
/// fans out to every port of the other. Otherwise channels are matched by
/// name (FL to FL, FR to FR); when no name matches, ports are paired in
/// order, and the longer side's extra ports are left alone.
///
/// # Examples
///
/// ```
/// use otto_audio_graph::pair_channels;
///
/// let out = [(1, Some("FL")), (2, Some("FR"))];
/// let inp = [(3, Some("FR")), (4, Some("FL"))];
/// assert_eq!(pair_channels(&out, &inp), vec![(1, 4), (2, 3)]);
/// ```
#[must_use]
pub fn pair_channels(
    outputs: &[(u32, Option<&str>)],
    inputs: &[(u32, Option<&str>)],
) -> Vec<(u32, u32)> {
    if outputs.is_empty() || inputs.is_empty() {
        return Vec::new();
    }
    if outputs.len() == 1 {
        return inputs.iter().map(|(i, _)| (outputs[0].0, *i)).collect();
    }
    if inputs.len() == 1 {
        return outputs.iter().map(|(o, _)| (*o, inputs[0].0)).collect();
    }
    let by_name: Vec<(u32, u32)> = outputs
        .iter()
        .filter_map(|(o, channel)| {
            let channel = (*channel)?;
            inputs
                .iter()
                .find(|(_, c)| *c == Some(channel))
                .map(|(i, _)| (*o, *i))
        })
        .collect();
    if !by_name.is_empty() {
        return by_name;
    }
    outputs
        .iter()
        .zip(inputs)
        .map(|((o, _), (i, _))| (*o, *i))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_links_by_channel_name() {
        let out = [(1, Some("FL")), (2, Some("FR"))];
        let inp = [(10, Some("FL")), (11, Some("FR"))];
        assert_eq!(pair_channels(&out, &inp), vec![(1, 10), (2, 11)]);
    }

    #[test]
    fn unmatched_names_link_in_order() {
        let out = [(1, Some("AUX0")), (2, Some("AUX1")), (3, Some("AUX2"))];
        let inp = [(10, Some("FL")), (11, Some("FR"))];
        assert_eq!(pair_channels(&out, &inp), vec![(1, 10), (2, 11)]);
    }

    #[test]
    fn mono_fans_out_to_every_channel() {
        let mic = [(1, Some("MONO"))];
        let speakers = [(10, Some("FL")), (11, Some("FR"))];
        assert_eq!(pair_channels(&mic, &speakers), vec![(1, 10), (1, 11)]);
        assert_eq!(pair_channels(&speakers, &mic), vec![(10, 1), (11, 1)]);
    }

    #[test]
    fn surround_into_stereo_takes_the_matching_channels() {
        let out = [
            (1, Some("FL")),
            (2, Some("FR")),
            (3, Some("FC")),
            (4, Some("LFE")),
        ];
        let inp = [(10, Some("FL")), (11, Some("FR"))];
        assert_eq!(pair_channels(&out, &inp), vec![(1, 10), (2, 11)]);
    }

    #[test]
    fn ports_without_channels_link_in_order() {
        let out = [(1, None), (2, None)];
        let inp = [(10, None), (11, None)];
        assert_eq!(pair_channels(&out, &inp), vec![(1, 10), (2, 11)]);
    }

    #[test]
    fn an_empty_side_links_nothing() {
        assert!(pair_channels(&[], &[(1, None)]).is_empty());
    }
}
