//! Type-coverage helpers for the party panel.
//!
//! Given the current live party, computes:
//! - which Gen III types are **present** on the team (at least one member has that type)
//! - which types the team is **collectively weak to** (≥1 member takes ×2 or more from them)
//!
//! Both sets are returned as small `Vec<u8>` of type IDs (0–16).

/// Number of distinct Gen III types (Normal through Dark, no Bird/???).
pub const NUM_TYPES: usize = 17;

/// Gen III damage multiplier table: `EFFECTIVENESS[atk][def]` is the
/// effectiveness of type `atk` against type `def`, expressed as eighths
/// (8 = ×1, 16 = ×2, 4 = ×½, 0 = ×0).
///
/// Row order: Normal Fighting Flying Poison Ground Rock Bug Ghost Steel
///            Fire  Water   Grass  Electric Psychic Ice  Dragon Dark
#[rustfmt::skip]
const EFFECTIVENESS: [[u8; NUM_TYPES]; NUM_TYPES] = [
//              Nml Fgt Fly Poi Gnd Rok Bug Gst Stl Fir Wat Grs Elc Psy Ice Drg Drk
/* Normal   */ [  8,  8,  8,  8,  8,  4,  8,  0,  4,  8,  8,  8,  8,  8,  8,  8,  8 ],
/* Fighting */ [ 16,  8,  4,  4,  8, 16,  4,  0, 16,  8,  8,  8,  8,  4,  8,  8, 16 ],
/* Flying   */ [  8, 16,  8,  8,  8,  4, 16,  8,  4,  8,  8, 16,  4,  8,  4,  8,  8 ],
/* Poison   */ [  8,  8,  8,  4,  4,  4,  8,  4,  0,  8,  8, 16,  8,  8,  8,  8,  8 ],
/* Ground   */ [  8,  8,  0,  16, 8,  16, 4,  8, 16, 16,  8,  4,  0,  8,  8,  8,  8 ],
/* Rock     */ [  8,  4,  16, 8,  4,  8, 16,  8,  4, 16,  8,  8,  8,  8, 16,  8,  8 ],
/* Bug      */ [  8,  4,  4,  4,  8,  8,  8,  4,  4,  4,  8, 16,  8, 16,  8,  8,  16],
/* Ghost    */ [  0,  8,  8,  8,  8,  8,  8, 16,  8,  8,  8,  8,  8, 16,  8,  8,  4 ],
/* Steel    */ [  8,  8,  8,  8,  8, 16,  8,  8,  4,  4,  4,  8,  4,  8, 16,  8,  8 ],
/* Fire     */ [  8,  8,  8,  8,  8,  4, 16,  8, 16,  4,  4, 16,  8,  8, 16,  4,  8 ],
/* Water    */ [  8,  8,  8,  8, 16, 16,  8,  8,  8, 16,  4,  4,  8,  8,  8,  4,  8 ],
/* Grass    */ [  8,  8,  4,  4, 16, 16,  4,  8,  4,  4, 16,  4,  8,  8,  8,  4,  8 ],
/* Electric */ [  8,  8, 16,  8,  0,  8,  8,  8, 16,  8, 16,  4,  4,  8,  8,  4,  8 ],
/* Psychic  */ [  8, 16,  8, 16,  8,  8,  8,  8,  4,  8,  8,  8,  8,  4,  8,  8,  0 ],
/* Ice      */ [  8,  8, 16,  8, 16,  8,  8,  8,  4,  4,  4, 16,  8,  8,  4, 16,  8 ],
/* Dragon   */ [  8,  8,  8,  8,  8,  8,  8,  8,  4,  8,  8,  8,  8,  8,  8, 16,  8 ],
/* Dark     */ [  8,  4,  8,  8,  8,  8,  8, 16,  8,  8,  8,  8,  8, 16,  8,  8,  4 ],
];

/// Summary of the live party's type coverage.
pub struct TypeCoverage {
    /// Type IDs present on at least one living (non-dead) party member.
    pub team_types: Vec<u8>,
    /// Attacking type IDs that hit at least one living team member for ×2 or more.
    pub team_weaknesses: Vec<u8>,
    /// Attacking type IDs that the team can hit super-effectively (×2+) with
    /// at least one of its own types.
    pub offensive_coverage: Vec<u8>,
}

/// Computes type coverage for a slice of `(type1, type2)` pairs, one per
/// living party member.
///
/// `types` should be built by calling [`fire_red_party_monitor::get_species_types`]
/// for each member that is alive (HP > 0 and not recorded as dead).
pub fn compute(types: &[(u8, u8)]) -> TypeCoverage {
    let mut team_types_set = [false; NUM_TYPES];
    for &(t1, t2) in types {
        if (t1 as usize) < NUM_TYPES {
            team_types_set[t1 as usize] = true;
        }
        if (t2 as usize) < NUM_TYPES && t2 != t1 {
            team_types_set[t2 as usize] = true;
        }
    }

    // Weaknesses: for each attacking type, does any member take ×2+?
    let mut weaknesses_set = [false; NUM_TYPES];
    for (atk, slot) in weaknesses_set.iter_mut().enumerate() {
        'member: for &(t1, t2) in types {
            let mult = effective_mult(atk as u8, t1, t2);
            if mult >= 16 {
                *slot = true;
                break 'member;
            }
        }
    }

    // Offensive coverage: for each defending type, can the team hit it ×2+?
    let mut coverage_set = [false; NUM_TYPES];
    for def in 0..NUM_TYPES {
        for atk in 0..NUM_TYPES {
            if !team_types_set[atk] {
                continue;
            }
            if EFFECTIVENESS[atk][def] >= 16 {
                coverage_set[def] = true;
                break;
            }
        }
    }

    TypeCoverage {
        team_types: team_types_set
            .iter()
            .enumerate()
            .filter(|&(_, &b)| b)
            .map(|(i, _)| i as u8)
            .collect(),
        team_weaknesses: weaknesses_set
            .iter()
            .enumerate()
            .filter(|&(_, &b)| b)
            .map(|(i, _)| i as u8)
            .collect(),
        offensive_coverage: coverage_set
            .iter()
            .enumerate()
            .filter(|&(_, &b)| b)
            .map(|(i, _)| i as u8)
            .collect(),
    }
}

/// Combined effectiveness multiplier (in eighths) when an attack of type
/// `atk` hits a target with types `def1`/`def2`.
///
/// For mono-type targets (`def1 == def2`) the multiplier is applied only
/// once, matching the Gen III engine behaviour.
fn effective_mult(atk: u8, def1: u8, def2: u8) -> u8 {
    let a = atk as usize;
    let d1 = def1 as usize;
    if a >= NUM_TYPES || d1 >= NUM_TYPES {
        return 8;
    }
    let m1 = EFFECTIVENESS[a][d1] as u16;
    if def1 == def2 {
        return m1 as u8;
    }
    let d2 = def2 as usize;
    if d2 >= NUM_TYPES {
        return m1 as u8;
    }
    let m2 = EFFECTIVENESS[a][d2] as u16;
    // Combined = (m1 * m2) / 8, all in eighths.
    ((m1 * m2) / 8) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_name_round_trip() {
        for id in 0..NUM_TYPES as u8 {
            let name = fire_red_party_monitor::type_name(id);
            assert!(!name.is_empty(), "type_name({id}) returned empty string");
            assert_ne!(name, "???", "type_name({id}) returned ???");
        }
    }

    #[test]
    fn effectiveness_table_known_self_matchups() {
        // Ghost and Dragon are super-effective vs themselves (Gen III)
        assert_eq!(EFFECTIVENESS[7][7], 16, "Ghost vs Ghost should be ×2");
        assert_eq!(EFFECTIVENESS[15][15], 16, "Dragon vs Dragon should be ×2");
        // Elemental types resist themselves
        assert_eq!(EFFECTIVENESS[9][9], 4, "Fire vs Fire should be ×½");
        assert_eq!(EFFECTIVENESS[10][10], 4, "Water vs Water should be ×½");
        assert_eq!(
            EFFECTIVENESS[12][12], 4,
            "Electric vs Electric should be ×½"
        );
        // Normal and Fighting are neutral to themselves
        assert_eq!(EFFECTIVENESS[0][0], 8, "Normal vs Normal should be ×1");
        assert_eq!(EFFECTIVENESS[1][1], 8, "Fighting vs Fighting should be ×1");
    }

    #[test]
    fn fire_vs_grass_is_super_effective() {
        assert_eq!(EFFECTIVENESS[9][11], 16);
    }

    #[test]
    fn normal_vs_ghost_is_immune() {
        assert_eq!(EFFECTIVENESS[0][7], 0);
    }

    #[test]
    fn water_vs_fire_is_super_effective() {
        assert_eq!(EFFECTIVENESS[10][9], 16);
    }

    #[test]
    fn compute_single_fire_type_has_fire_in_team_types() {
        let cov = compute(&[(9, 9)]);
        assert!(cov.team_types.contains(&9), "Fire should be in team types");
    }

    #[test]
    fn compute_fire_type_is_weak_to_water() {
        let cov = compute(&[(9, 9)]);
        assert!(
            cov.team_weaknesses.contains(&10),
            "Fire team should be weak to Water (10)"
        );
    }

    #[test]
    fn compute_fire_type_covers_grass() {
        let cov = compute(&[(9, 9)]);
        assert!(
            cov.offensive_coverage.contains(&11),
            "Fire covers Grass (11)"
        );
    }

    #[test]
    fn compute_empty_party_returns_empty_sets() {
        let cov = compute(&[]);
        assert!(cov.team_types.is_empty());
        assert!(cov.team_weaknesses.is_empty());
        assert!(cov.offensive_coverage.is_empty());
    }

    #[test]
    fn effective_mult_fire_grass() {
        assert_eq!(effective_mult(9, 11, 11), 16);
    }

    #[test]
    fn effective_mult_normal_ghost_is_immune() {
        assert_eq!(effective_mult(0, 7, 7), 0);
    }

    #[test]
    fn effective_mult_dual_type_compounds() {
        // Water vs Rock/Fire: ×2 vs Rock(5) × ×2 vs Fire(9) = ×4.
        // Represented as 32 eighths (32/8 = ×4).
        let mult = effective_mult(10, 5, 9);
        assert_eq!(mult, 32, "Water vs Rock/Fire should be ×4 (32 eighths)");
    }

    // ── Regression tests for corrected table cells ─────────────────────────

    #[test]
    fn dark_vs_psychic_is_super_effective() {
        // Was incorrectly coded as 0 (immune). Dark is ×2 vs Psychic in Gen III.
        assert_eq!(EFFECTIVENESS[16][13], 16, "Dark vs Psychic should be ×2");
    }

    #[test]
    fn flying_vs_ground_is_neutral() {
        // Was incorrectly coded as 0 (immune). The immunity only runs the other
        // direction: Ground moves cannot hit Flying-type Pokémon, but a Flying-type
        // move has no special interaction against a Ground-type target.
        assert_eq!(EFFECTIVENESS[2][4], 8, "Flying vs Ground should be ×1");
    }

    #[test]
    fn flying_vs_electric_is_not_very_effective() {
        // Was incorrectly coded as 16 (×2 SE) — a transposition of the
        // Electric→Flying advantage. Flying attacks Electric for ×½.
        assert_eq!(EFFECTIVENESS[2][12], 4, "Flying vs Electric should be ×½");
    }

    #[test]
    fn dark_covers_psychic_in_compute() {
        let cov = compute(&[(16, 16)]); // mono Dark team
        // EFFECTIVENESS[16][13] was incorrectly 0 (immune); fix set it to 16 (×2).
        // This assertion directly verifies the corrected table cell.
        assert_eq!(
            EFFECTIVENESS[16][13], 16,
            "Dark vs Psychic should be ×2 (EFFECTIVENESS[16][13])"
        );
        assert!(
            cov.offensive_coverage.contains(&13),
            "Dark team should cover Psychic (13)"
        );
    }

    #[test]
    fn ghost_vs_fighting_is_neutral() {
        // Was incorrectly coded as 0 (immune) — a Gen I holdover where Ghost
        // moves could not hit Normal (type 0) or Fighting (type 1) targets.
        // In Gen III, only Normal is immune to Ghost; Fighting takes ×1.
        assert_eq!(EFFECTIVENESS[7][1], 8, "Ghost vs Fighting should be ×1");
    }
}
