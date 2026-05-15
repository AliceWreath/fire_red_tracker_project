use std::{error::Error, io::{BufRead, BufReader}};

use rusqlite::{Connection, Result};

pub fn initialize_location_database(path: &str, csv_path: &str) -> Result<()> {
    //Open or create the database path
    let conn = Connection::open(path)?;

    //Create a table
    match conn.execute(
        "CREATE TABLE IF NOT EXISTS location_data (
        location_id_group_and_number INTEGER PRIMARY KEY UNIQUE,
        location_name TEXT UNIQUE,
        first_encounter_p1 INTEGER,
        caught_p1 BOOL,
        first_encounter_p2 INTEGER,
        caught_p2 BOOL
        )", [],
    ) {
        Ok(_) => (),
        Err(e) => println!("create failed: {}", e),
    };

    fill_location_info(&conn, csv_path)
}

pub fn initialize_player_pokemon_database(path: &str) -> Result<()> {
    let conn = Connection::open(path)?;

    conn.execute("
        CREATE TABLE IF NOT EXISTS caught_pokemon_p1 (
        catch_order INTEGER PRIMARY KEY AUTOINCREMENT,
        pokemon_id INTEGER,
        pokemon_species STRING,
        pokemon_nickname STRING,
        pokemon_caught_level INTEGER,
        pokemon_caught_location_id INTEGER,
        pokemon_status STRING,
        pokemon_level INTEGER,
        pokemon_hp INTEGER,
        pokemon_maxhp INTEGER,
        pokemon_attack INTEGER,
        pokemon_defense INTEGER,
        pokemon_speed INTEGER,
        pokemon_sp_attack INTEGER,
        pokemon_sp_defense INTEGER,
        pokemon_held_item_id INTEGER,
        pokemon_experience INTEGER,
        pokemon_friendship INTEGER,
        pokemon_move_1_id INTEGER,
        pokemon_move_2_id INTEGER,
        pokemon_move_3_id INTEGER,
        pokemon_move_4_id INTEGER,        
        pokemon_move_1_pp INTEGER,
        pokemon_move_2_pp INTEGER,
        pokemon_move_3_pp INTEGER,
        pokemon_move_4_pp INTEGER,
        pokemon_hp_ev INTEGER,
        pokemon_attack_ev INTEGER,
        pokemon_defense_ev INTEGER,
        pokemon_speed_ev INTEGER,
        pokemon_sp_attack_ev INTEGER,
        pokemon_sp_defense_ev INTEGER,
        pokemon_cool INTEGER,
        pokemon_beauty INTEGER,
        pokemon_cute INTEGER,
        pokemon_smart INTEGER,
        pokemon_tough INTEGER,
        pokemon_sheen INTEGER,
        pokemon_hp_iv INTEGER,
        pokemon_attack_iv INTEGER,
        pokemon_defense_iv INTEGER,
        pokemon_speed_iv INTEGER,
        pokemon_sp_attack_iv INTEGER,
        pokemon_sp_defense_iv INTEGER
    )", [])?;
    Ok(())
}

fn fill_location_info(conn: &Connection, csv_file_path: &str) -> Result<()> {
    let file = std::fs::File::open(csv_file_path).expect("Unable to open csv");
    let reader = BufReader::new(file);
    let mut stmt = conn.prepare("INSERT OR IGNORE INTO location_data (location_name, location_id_group_and_number) VALUES ($1, $2)")?;
    for (_, line) in reader.lines().enumerate() {
        let line = line.unwrap();
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() >= 2 {
            let val = u16::from_str_radix(parts[1].strip_prefix("0x").or_else(|| parts[1].strip_prefix("0X")).unwrap().trim(), 16).unwrap();
            stmt.execute([parts[0], format!("{val}").as_str()])?;
        }
    }

    Ok(())
}

pub fn is_first_encounter(conn: &Connection, species_id: u16) -> Result<bool, Box<dyn Error>> {
    let mut stmt = conn.prepare("
        SELECT caught_p1
        FROM location_data
        WHERE first_encounter_p1 = ?1")?;
    let result_p1 = stmt.query_map([species_id], |row| { row.get::<_, bool>(0)})?;
    
    let mut stmt2 = conn.prepare("
        SELECT caught_p2
        FROM location_data
        WHERE first_encounter_p1 = ?1")?;
    let result_p2 = stmt2.query_map([species_id], |row| { row.get::<_, bool>(0)})?;

    let mut first_enc: bool = false;
    for caught in result_p1 {
        match caught {
            Ok(caught) => first_enc = if caught == true { false } else { true },
            Err(e) => return Err(e.into()),
        };
    }
    if first_enc {
        return Ok(true);
    }
    for caught in result_p2 {
        match caught {
            Ok(caught) => first_enc = if caught == true { false } else { true },
            Err(e) => return Err(e.into()),
        };
    }
    
    Ok(first_enc)
}

pub fn is_duplicate_encounter(conn: &Connection, species_id: u16) -> Result<bool, Box<dyn Error>> {
    let mut stmt = conn.prepare("
        SELECT caught_p1, caught_p2
        FROM location_data
        WHERE first_encounter_p1 = ?1 or first_encounter_p2 = ?1")?;
    let result = stmt.query_map([species_id.to_string().as_str()], |row| row.get::<_, bool>(0))?;
    for caught in result {
        match caught {
            Ok(caught) => return Ok(caught),
            Err(e) => return Err(e.into()),
        };
    }
    
    Ok(false)
}

pub fn update_location_first_encounter(path: &str, location_id: u16, player_number: u8, species_id: u16) -> Result<(), Box<dyn Error>> {
    let conn = Connection::open(path)?;
    let first_encounter = is_first_encounter(&conn, species_id)?;
    if first_encounter == false {
        return Ok(());
    }
    if is_duplicate_encounter(&conn, species_id)? {
        return Ok(());
    }

    conn.execute("
        UPDATE location_data
        SET ?1 = ?2
        WHERE location_id_group_and_number = ?3",
        [if player_number == 1 { "first_encounter_p1" } else { "first_encounter_p2" }, species_id.to_string().as_str(), location_id.to_string().as_str()]
    )?;

    Ok(())
}

pub fn reset_database(path: &str) -> Result<()> {
    let conn = Connection::open(path)?;
    conn.execute("
    UPDATE location_data 
    SET first_encounter_p1 = -1, caught_p1 = false, first_encounter_p2 = -1, caught_p2 = false", [])?;

    Ok(())
}

#[cfg(test)]
mod tests {
    const DB_PATH: &str = "/home/alice/data/projects/rust/fire_red_project/db.db";
    const CSV_PATH: &str = "/home/alice/data/projects/rust/fire_red_project/locations.csv";
    use super::*;

    #[test]
    fn build_location_database_test() -> Result<(), Box<dyn Error>> {
        reset_database(DB_PATH)?;

        let conn = Connection::open(DB_PATH)?;
        initialize_location_database(DB_PATH, CSV_PATH)?;
        initialize_player_pokemon_database(DB_PATH)?;

        let mut stmt = conn.prepare(
            "SELECT location_name
            FROM location_data
            WHERE location_id_group_and_number IN (?1, ?2, ?3, ?4)"
        )?;

        let rows = stmt.query_map([768, 769, 1024, 0x611], |row| {
            Ok((
                row.get::<_, String>(0)?,
            ))
        })?;

        let rows:Vec<String> = rows.map(|r| r.unwrap().0).collect();

        assert_eq!(rows[0].to_ascii_uppercase(), "PALLET TOWN");
        assert_eq!(rows[3].to_ascii_uppercase(), "GREEN PATH");
        
        let res = update_location_first_encounter(DB_PATH, 0x606, 1, 0x100);
        match res {
            Ok(_) => return Ok(()),
            Err(e) => return Err(e.into()),
        }
    }
}