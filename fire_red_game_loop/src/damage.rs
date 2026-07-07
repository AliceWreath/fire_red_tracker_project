//! Gen III damage calculator for the live battle panel.
//!
//! Computes, for every party member, the damage range of each equipped move
//! against the enemy currently loaded in `gEnemyParty[0]`, using the ROM's
//! `gBattleMoves` table for power/type and the standard Gen III formula:
//!
//! ```text
//! base = floor(floor(floor(2·L/5 + 2) · power · A / D) / 50) + 2
//! damage = base · STAB(1.5) · type1 · type2, rolled 85–100%
//! ```
//!
//! Gen III has no physical/special split per move — the category is decided
//! by the move's *type*. Final stats read from the party structs already
//! include nature, IVs, and EVs, so no stat math is needed. Modeled on top of
//! the base formula: STAB, dual-type effectiveness, burn halving physical
//! attack, and the defender's Levitate ability. Not modeled: other abilities,
//! held items, screens, weather, stat stages, and variable-power moves
//! (which the ROM stores with power 1).

use fire_red_party_monitor::Pokemon;
use fire_red_states::{DamageAttacker, DamageMove, DamagePanel};

/// Size of one `gBattleMoves` entry: effect, power, type, accuracy, pp,
/// secondary-effect chance, target, priority, flags, 3 padding bytes.
const MOVE_ENTRY_SIZE: usize = 12;

/// Burn status bit in the party struct's status bitmask.
const STATUS_BURN: u32 = 0x10;

/// Gen III internal type IDs (indices into the effectiveness chart).
const TYPE_COUNT: usize = 18;
const TYPE_GROUND: u8 = 4;
const TYPE_MYSTERY: u8 = 9;

/// Reads `(power, type)` for a move from the ROM's `gBattleMoves` table.
/// Returns `(0, 0)` when the entry is out of bounds.
pub(crate) fn move_power_type(rom: &[u8], move_data_addr: usize, move_id: u16) -> (u8, u8) {
    let off = move_data_addr + move_id as usize * MOVE_ENTRY_SIZE;
    if off + 2 >= rom.len() {
        return (0, 0);
    }
    (rom[off + 1], rom[off + 2])
}

/// True when a Gen III move of this type uses Attack/Defense; false for
/// Sp. Attack/Sp. Defense. (The physical/special split is per-type until
/// Gen IV.) Physical: Normal, Fighting, Flying, Poison, Ground, Rock, Bug,
/// Ghost, Steel. Special: Fire, Water, Grass, Electric, Psychic, Ice,
/// Dragon, Dark.
pub(crate) fn is_physical(move_type: u8) -> bool {
    move_type <= 8
}

/// Type effectiveness ×10: 0 = immune, 5 = not very effective, 10 = neutral,
/// 20 = super effective. Gen III chart (Steel still resists Ghost and Dark).
pub(crate) fn effectiveness_x10(attack_type: u8, defend_type: u8) -> u16 {
    // Rows: attacker type; entries: (defender type, multiplier ×10).
    const CHART: [&[(u8, u16)]; TYPE_COUNT] = [
        /* Normal   */ &[(5, 5), (8, 5), (7, 0)],
        /* Fighting */ &[(0, 20), (5, 20), (8, 20), (15, 20), (17, 20), (2, 5), (3, 5), (6, 5), (14, 5), (7, 0)],
        /* Flying   */ &[(1, 20), (6, 20), (12, 20), (5, 5), (8, 5), (13, 5)],
        /* Poison   */ &[(12, 20), (3, 5), (4, 5), (5, 5), (7, 5), (8, 0)],
        /* Ground   */ &[(3, 20), (5, 20), (8, 20), (10, 20), (13, 20), (6, 5), (12, 5), (2, 0)],
        /* Rock     */ &[(2, 20), (6, 20), (10, 20), (15, 20), (1, 5), (4, 5), (8, 5)],
        /* Bug      */ &[(12, 20), (14, 20), (17, 20), (1, 5), (2, 5), (3, 5), (7, 5), (8, 5), (10, 5)],
        /* Ghost    */ &[(7, 20), (14, 20), (0, 0), (17, 5), (8, 5)],
        /* Steel    */ &[(5, 20), (15, 20), (10, 5), (11, 5), (13, 5), (8, 5)],
        /* Mystery  */ &[],
        /* Fire     */ &[(6, 20), (8, 20), (12, 20), (15, 20), (5, 5), (10, 5), (11, 5), (16, 5)],
        /* Water    */ &[(4, 20), (5, 20), (10, 20), (11, 5), (12, 5), (16, 5)],
        /* Grass    */ &[(4, 20), (5, 20), (11, 20), (2, 5), (3, 5), (6, 5), (8, 5), (10, 5), (12, 5), (16, 5)],
        /* Electric */ &[(2, 20), (11, 20), (4, 0), (12, 5), (13, 5), (16, 5)],
        /* Psychic  */ &[(1, 20), (3, 20), (14, 5), (8, 5), (17, 0)],
        /* Ice      */ &[(2, 20), (4, 20), (12, 20), (16, 20), (8, 5), (10, 5), (11, 5), (15, 5)],
        /* Dragon   */ &[(16, 20), (8, 5)],
        /* Dark     */ &[(7, 20), (14, 20), (1, 5), (17, 5), (8, 5)],
    ];
    if attack_type as usize >= TYPE_COUNT || defend_type as usize >= TYPE_COUNT {
        return 10;
    }
    CHART[attack_type as usize]
        .iter()
        .find(|(t, _)| *t == defend_type)
        .map(|(_, m)| *m)
        .unwrap_or(10)
}

/// Combined effectiveness ×100 of a move type against a (possibly dual-typed)
/// defender. `type2 == type1` means single-typed.
pub(crate) fn combined_effectiveness_x100(move_type: u8, type1: u8, type2: u8) -> u16 {
    let e1 = effectiveness_x10(move_type, type1);
    let e2 = if type2 != type1 {
        effectiveness_x10(move_type, type2)
    } else {
        10
    };
    e1 * e2
}

/// Gen III damage range for one move: `(min, max)` at the 85% and 100% rolls.
/// `attack`/`defense` must already be the stats matching the move's category.
pub(crate) fn damage_range(
    level: u8,
    power: u8,
    attack: u16,
    defense: u16,
    stab: bool,
    effectiveness_x100: u16,
) -> (u16, u16) {
    if power == 0 || effectiveness_x100 == 0 || defense == 0 {
        return (0, 0);
    }
    let mut dmg: u32 = (2 * level as u32 / 5 + 2) * power as u32 * attack as u32 / defense as u32;
    dmg = dmg / 50 + 2;
    if stab {
        dmg = dmg * 15 / 10;
    }
    dmg = dmg * effectiveness_x100 as u32 / 100;
    let max = dmg.min(u16::MAX as u32) as u16;
    let min = (dmg * 85 / 100).min(u16::MAX as u32) as u16;
    (min.max(1), max.max(1))
}

/// Fixed-damage moves the ROM stores with power 1. Returns the flat damage
/// dealt (type effectiveness still applies as immune-or-hit in Gen III).
fn fixed_damage(move_name: &str, attacker_level: u8) -> Option<u16> {
    match move_name {
        "Seismic Toss" | "Night Shade" => Some(attacker_level as u16),
        "Dragon Rage" => Some(40),
        "Sonicboom" | "Sonic Boom" => Some(20),
        _ => None,
    }
}

/// Builds the live damage panel: every party member's equipped moves against
/// `enemy`. `rom`/`move_data_addr` locate the `gBattleMoves` table.
pub fn build_panel(
    party: &[Pokemon],
    enemy: &Pokemon,
    is_trainer: bool,
    rom: &[u8],
    move_data_addr: usize,
) -> DamagePanel {
    let enemy_species = enemy.box_mon.secure.growth.species;
    let (enemy_type1, enemy_type2) =
        fire_red_party_monitor::species_type_static(enemy_species);
    let enemy_levitates = enemy.box_mon.ability_string == "Levitate";

    let attackers: Vec<DamageAttacker> = party
        .iter()
        .filter(|p| p.box_mon.personality != 0)
        .map(|p| {
            let (a_type1, a_type2) =
                fire_red_party_monitor::species_type_static(p.box_mon.secure.growth.species);
            let burned = p.status & STATUS_BURN != 0;
            let moves: Vec<DamageMove> = p
                .box_mon
                .secure
                .attack
                .moves
                .iter()
                .zip(p.box_mon.secure.attack.pp.iter())
                .filter(|(id, _)| **id != 0)
                .map(|(&id, &pp)| {
                    let (power, move_type) = move_power_type(rom, move_data_addr, id);
                    let name = fire_red_database::move_name(id).to_string();
                    let eff = combined_effectiveness_x100(move_type, enemy_type1, enemy_type2);
                    // Levitate grants immunity to Ground on top of the chart.
                    let eff = if enemy_levitates && move_type == TYPE_GROUND { 0 } else { eff };
                    let stab = move_type != TYPE_MYSTERY
                        && (move_type == a_type1 || move_type == a_type2);
                    let (min, max) = if let Some(flat) = fixed_damage(&name, p.level) {
                        // Fixed-damage moves ignore STAB/effectiveness but
                        // still miss entirely against immune types.
                        if eff == 0 { (0, 0) } else { (flat, flat) }
                    } else if power <= 1 {
                        // Status moves (0) and variable-power moves (1) that
                        // the calculator does not model.
                        (0, 0)
                    } else {
                        let (attack, defense) = if is_physical(move_type) {
                            let atk = if burned { p.attack / 2 } else { p.attack };
                            (atk, enemy.defense)
                        } else {
                            (p.sp_attack, enemy.sp_defense)
                        };
                        damage_range(p.level, power, attack, defense, stab, eff)
                    };
                    DamageMove {
                        name,
                        move_type,
                        power,
                        pp,
                        min,
                        max,
                        effectiveness: eff,
                        stab,
                        guaranteed_ko: min > 0 && min >= enemy.hp,
                    }
                })
                .collect();
            DamageAttacker {
                nickname: p.box_mon.nickname_string.clone(),
                species_name: p.box_mon.secure.growth.species_string.clone(),
                level: p.level,
                hp: p.hp,
                max_hp: p.max_hp,
                moves,
            }
        })
        .collect();

    DamagePanel {
        enemy_species: enemy.box_mon.secure.growth.species_string.clone(),
        enemy_level: enemy.level,
        enemy_hp: enemy.hp,
        enemy_max_hp: enemy.max_hp,
        enemy_type1,
        enemy_type2,
        is_trainer,
        attackers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── effectiveness chart ──────────────────────────────────────────────

    #[test]
    fn effectiveness_known_matchups() {
        assert_eq!(effectiveness_x10(13, 11), 20, "Electric vs Water");
        assert_eq!(effectiveness_x10(13, 4), 0, "Electric vs Ground");
        assert_eq!(effectiveness_x10(0, 7), 0, "Normal vs Ghost");
        assert_eq!(effectiveness_x10(1, 7), 0, "Fighting vs Ghost");
        assert_eq!(effectiveness_x10(10, 11), 5, "Fire vs Water");
        assert_eq!(effectiveness_x10(10, 12), 20, "Fire vs Grass");
        assert_eq!(effectiveness_x10(0, 0), 10, "Normal vs Normal");
        // Gen III specifics: Steel resists Ghost and Dark.
        assert_eq!(effectiveness_x10(7, 8), 5, "Ghost vs Steel");
        assert_eq!(effectiveness_x10(17, 8), 5, "Dark vs Steel");
    }

    #[test]
    fn dual_type_effectiveness_multiplies() {
        // Electric vs Water/Flying (Gyarados) = 4×.
        assert_eq!(combined_effectiveness_x100(13, 11, 2), 400);
        // Grass vs Water/Ground (Quagsire) = 4×.
        assert_eq!(combined_effectiveness_x100(12, 11, 4), 400);
        // Fighting vs Ghost/anything = immune.
        assert_eq!(combined_effectiveness_x100(1, 7, 3), 0);
        // Single-typed defender must not double-count its one type.
        assert_eq!(combined_effectiveness_x100(13, 11, 11), 200);
    }

    #[test]
    fn every_chart_row_is_within_type_ids() {
        for atk in 0..18u8 {
            for def in 0..18u8 {
                let e = effectiveness_x10(atk, def);
                assert!(matches!(e, 0 | 5 | 10 | 20), "bad entry {atk}->{def}: {e}");
            }
        }
        assert_eq!(effectiveness_x10(30, 0), 10, "out-of-range attacker is neutral");
    }

    // ── physical/special split ───────────────────────────────────────────

    #[test]
    fn gen3_split_is_by_type() {
        for t in [0u8, 1, 2, 3, 4, 5, 6, 7, 8] {
            assert!(is_physical(t), "type {t} should be physical");
        }
        for t in [10u8, 11, 12, 13, 14, 15, 16, 17] {
            assert!(!is_physical(t), "type {t} should be special");
        }
    }

    // ── damage formula ───────────────────────────────────────────────────

    #[test]
    fn damage_formula_reference_case() {
        // L50, power 95 (Thunderbolt), 120 SpA vs 80 SpD, STAB, 2× effective:
        // base = floor(floor(22 * 95 * 120 / 80) / 50) + 2 = floor(3135/50)+2 = 64
        // 64 * 1.5 = 96, ×2 = 192; min = floor(192*0.85) = 163.
        let (min, max) = damage_range(50, 95, 120, 80, true, 200);
        assert_eq!(max, 192);
        assert_eq!(min, 163);
    }

    #[test]
    fn damage_zero_on_immunity_or_status() {
        assert_eq!(damage_range(50, 95, 120, 80, true, 0), (0, 0));
        assert_eq!(damage_range(50, 0, 120, 80, false, 100), (0, 0));
    }

    #[test]
    fn damage_is_at_least_one_when_it_hits() {
        let (min, max) = damage_range(2, 10, 5, 200, false, 25);
        assert!(min >= 1 && max >= 1, "connecting hits deal at least 1: {min}-{max}");
    }

    // ── move table reader ────────────────────────────────────────────────

    #[test]
    fn move_power_type_reads_table_entry() {
        // Move 2 lives at offset 24: effect, power=40, type=1 (Fighting), ...
        let mut rom = vec![0u8; 64];
        rom[24] = 0; // effect
        rom[25] = 40; // power
        rom[26] = 1; // type
        assert_eq!(move_power_type(&rom, 0, 2), (40, 1));
        assert_eq!(move_power_type(&rom, 0, 999), (0, 0), "out of bounds");
    }

    // ── fixed damage ─────────────────────────────────────────────────────

    #[test]
    fn fixed_damage_moves() {
        assert_eq!(fixed_damage("Seismic Toss", 31), Some(31));
        assert_eq!(fixed_damage("Night Shade", 12), Some(12));
        assert_eq!(fixed_damage("Dragon Rage", 5), Some(40));
        assert_eq!(fixed_damage("Tackle", 50), None);
    }

    // ── end-to-end panel ─────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    fn mon(species: u16, level: u8, hp: u16, atk: u16, spa: u16, def: u16, spd: u16, moves: [u16; 4]) -> Pokemon {
        let mut p = Pokemon::default();
        p.box_mon.personality = 1;
        p.box_mon.secure.growth.species = species;
        p.box_mon.secure.attack.moves = moves;
        p.box_mon.secure.attack.pp = [10, 10, 10, 10];
        p.level = level;
        p.hp = hp;
        p.max_hp = hp;
        p.attack = atk;
        p.sp_attack = spa;
        p.defense = def;
        p.sp_defense = spd;
        p
    }

    #[test]
    fn build_panel_produces_lines_per_equipped_move() {
        // Synthetic gBattleMoves: move 1 = 40-power Normal, move 2 = status.
        let mut rom = vec![0u8; 64];
        rom[12 + 1] = 40; // move 1 power
        rom[12 + 2] = 0; // move 1 type: Normal
        // move 2: power 0 (status) — bytes already zero.

        let attacker = mon(25, 20, 50, 30, 30, 25, 25, [1, 2, 0, 0]);
        let enemy = mon(19, 18, 40, 20, 20, 22, 22, [1, 0, 0, 0]);

        let panel = build_panel(&[attacker], &enemy, false, &rom, 0);
        assert_eq!(panel.attackers.len(), 1);
        let moves = &panel.attackers[0].moves;
        assert_eq!(moves.len(), 2, "two equipped slots -> two lines");
        assert!(moves[0].max > 0, "damaging move has a range");
        assert_eq!(moves[1].max, 0, "status move has no range");
        assert!(!panel.is_trainer);
        assert_eq!(panel.enemy_level, 18);
    }
}
