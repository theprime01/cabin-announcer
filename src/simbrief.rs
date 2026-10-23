//! Fills the flight details from your latest SimBrief flight plan.

use serde_json::Value;

use crate::config::Flight;

fn text(v: &Value, path: &[&str]) -> String {
    let mut cur = v;
    for p in path {
        cur = &cur[*p];
    }
    match cur {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// "LONDON HEATHROW" -> "London Heathrow".
fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Airport names often end in "Intl" or "Airport", which nobody says on the PA.
fn place(airport: &str) -> String {
    let words: Vec<&str> = airport.split_whitespace().collect();
    let keep: Vec<&str> = words
        .iter()
        .copied()
        .filter(|w| !matches!(w.to_ascii_lowercase().as_str(), "intl" | "international" | "airport" | "arpt"))
        .collect();
    title_case(&keep.join(" "))
}

pub fn parse(ofp: &Value) -> Result<Flight, String> {
    if text(ofp, &["fetch", "status"]).to_ascii_lowercase().contains("error") {
        return Err(text(ofp, &["fetch", "status"]));
    }
    let icao = text(ofp, &["general", "icao_airline"]);
    let number = text(ofp, &["general", "flight_number"]);
    Ok(Flight {
        airline_icao: icao.clone(),
        flight_number: format!("{icao}{number}"),
        origin: place(&text(ofp, &["origin", "name"])),
        destination: place(&text(ofp, &["destination", "name"])),
        // Scheduled off-block time, Unix seconds.
        scheduled_departure: text(ofp, &["times", "sched_out"]).parse::<i64>().ok().filter(|&t| t > 0),
    })
}

pub fn fetch(user: &str) -> Result<Flight, String> {
    let user = user.trim();
    if user.is_empty() {
        return Err("enter your SimBrief username first".into());
    }
    // A numeric value is a SimBrief pilot ID, anything else a username.
    let param = if user.chars().all(|c| c.is_ascii_digit()) { "userid" } else { "username" };
    let ofp: Value = ureq::get("https://www.simbrief.com/api/xml.fetcher.php")
        .query(param, user)
        .query("json", "1")
        .call()
        .map_err(|e| format!("SimBrief: {e}"))?
        .into_json()
        .map_err(|e| format!("SimBrief: {e}"))?;
    parse(&ofp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_an_ofp() {
        let ofp = serde_json::json!({
            "fetch": {"status": "Success"},
            "general": {"icao_airline": "BAW", "flight_number": "117", "initial_altitude": "36000"},
            "origin": {"name": "LONDON HEATHROW"},
            "destination": {"name": "JOHN F KENNEDY INTL"},
            "times": {"est_block": "28200", "sched_out": "1793001600"}
        });
        let f = parse(&ofp).unwrap();
        assert_eq!(f.airline_icao, "BAW");
        assert_eq!(f.flight_number, "BAW117");
        assert_eq!(f.origin, "London Heathrow");
        assert_eq!(f.destination, "John F Kennedy");
        assert_eq!(f.scheduled_departure, Some(1_793_001_600));
    }
}
