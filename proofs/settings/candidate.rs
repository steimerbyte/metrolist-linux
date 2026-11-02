use vstd::prelude::*;

verus! {

// `Settings::default()` maps the upstream seed `#ED5564` to RGB channels.
pub open spec fn seed_rgb_spec(color: u32) -> (u8, u8, u8) {
    (
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
    )
}

pub fn seed_rgb(color: u32) -> (r: (u8, u8, u8))
    ensures r == seed_rgb_spec(color),
{
    (
        ((color >> 16) & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        (color & 0xFF) as u8,
    )
}

proof fn lemma_seed_default()
    ensures seed_rgb_spec(0x00ED_5564) == (0xEDu8, 0x55u8, 0x64u8),
{
    assert(((((0x00ED_5564u32 >> 16) & 0xFF) as u8) == 0xEDu8)
        && ((((0x00ED_5564u32 >> 8) & 0xFF) as u8) == 0x55u8)
        && (((0x00ED_5564u32 & 0xFF) as u8) == 0x64u8)) by (bit_vector);
}

/// The alpha byte never reaches a channel: only the low 24 bits matter.
proof fn lemma_seed_alpha_ignored(c: u32)
    ensures seed_rgb_spec(c) == seed_rgb_spec(c & 0x00FF_FFFF),
{
    assert((((c >> 16) & 0xFF) == (((c & 0x00FF_FFFF) >> 16) & 0xFF))
        && (((c >> 8) & 0xFF) == (((c & 0x00FF_FFFF) >> 8) & 0xFF))
        && (((c & 0xFF) == ((c & 0x00FF_FFFF) & 0xFF)))) by (bit_vector);
}

/// Channels reconstruct the low 24 bits exactly: no information is dropped.
proof fn lemma_seed_reconstruct(c: u32)
    ensures
        (seed_rgb_spec(c).0 as u32) << 16
            | (seed_rgb_spec(c).1 as u32) << 8
            | (seed_rgb_spec(c).2 as u32) == c & 0x00FF_FFFF,
{
    assert(
        ((((c >> 16) & 0xFF) as u8) as u32) << 16
            | ((((c >> 8) & 0xFF) as u8) as u32) << 8
            | (((c & 0xFF) as u8) as u32) == c & 0x00FF_FFFF
    ) by (bit_vector);
}

} // verus!
