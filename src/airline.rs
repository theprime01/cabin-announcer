//! Recognises the airline you're flying for, from the SimBrief plan or from
//! what the livery reports to the sim (livery name, ATC airline, title).

pub struct Airline {
    pub icao: &'static str,
    pub name: &'static str,
    /// Other ICAO codes flying under the same brand (sister airlines/AOCs).
    pub also: &'static [&'static str],
    /// Lower-case words that identify it in livery names and ATC callsigns.
    pub words: &'static [&'static str],
}

pub const AIRLINES: &[Airline] = &[
    Airline { icao: "AAL", name: "American Airlines", also: &[], words: &["american airlines", "american"] },
    Airline { icao: "ACA", name: "Air Canada", also: &["ROU"], words: &["air canada"] },
    Airline { icao: "AFR", name: "Air France", also: &[], words: &["air france", "airfrans"] },
    Airline { icao: "ANA", name: "All Nippon Airways", also: &[], words: &["all nippon"] },
    Airline { icao: "BAW", name: "British Airways", also: &["SHT", "EFW"], words: &["british airways", "speedbird"] },
    Airline { icao: "CPA", name: "Cathay Pacific", also: &[], words: &["cathay"] },
    Airline { icao: "DAL", name: "Delta", also: &[], words: &["delta"] },
    Airline { icao: "DLH", name: "Lufthansa", also: &["CLH"], words: &["lufthansa"] },
    Airline { icao: "EIN", name: "Aer Lingus", also: &["EAI"], words: &["aer lingus", "shamrock"] },
    Airline { icao: "ETD", name: "Etihad", also: &[], words: &["etihad"] },
    Airline { icao: "EZY", name: "easyJet", also: &["EJU", "EZS"], words: &["easyjet"] },
    Airline { icao: "FDX", name: "FedEx", also: &[], words: &["fedex"] },
    Airline { icao: "FIN", name: "Finnair", also: &[], words: &["finnair"] },
    Airline { icao: "ICE", name: "Icelandair", also: &[], words: &["icelandair"] },
    Airline { icao: "JAL", name: "Japan Airlines", also: &[], words: &["japan airlines", "japanair"] },
    Airline { icao: "KLM", name: "KLM", also: &[], words: &["klm"] },
    Airline {
        icao: "NAX",
        name: "Norwegian",
        // Norwegian Air Shuttle and its AOCs: Norwegian Air Norway, Air Sweden,
        // Air International, Air Sweden AOC.
        also: &["NAN", "NSZ", "IBK", "NOZ", "NSW", "DAN"],
        words: &["norwegian", "nor shuttle", "norshuttle", "rednose", "nortrans"],
    },
    Airline { icao: "QFA", name: "Qantas", also: &[], words: &["qantas"] },
    Airline { icao: "QTR", name: "Qatar Airways", also: &[], words: &["qatar"] },
    Airline { icao: "RYR", name: "Ryanair", also: &["RUK", "MAY", "LDA"], words: &["ryanair"] },
    Airline { icao: "SAS", name: "Scandinavian", also: &["SZS", "CKS"], words: &["scandinavian", "sas "] },
    Airline { icao: "SIA", name: "Singapore Airlines", also: &[], words: &["singapore"] },
    Airline { icao: "SWA", name: "Southwest", also: &[], words: &["southwest"] },
    Airline { icao: "SWR", name: "Swiss", also: &[], words: &["swiss"] },
    Airline { icao: "THY", name: "Turkish Airlines", also: &[], words: &["turkish"] },
    Airline { icao: "TOM", name: "TUI", also: &["TUI", "JAF", "TFL"], words: &["tui ", "tomson", "tomjet"] },
    Airline { icao: "UAE", name: "Emirates", also: &[], words: &["emirates"] },
    Airline { icao: "UAL", name: "United", also: &[], words: &["united airlines", "united"] },
    Airline { icao: "UPS", name: "UPS", also: &[], words: &["ups "] },
    Airline { icao: "VIR", name: "Virgin Atlantic", also: &[], words: &["virgin atlantic", "virgin"] },
    Airline { icao: "WJA", name: "WestJet", also: &[], words: &["westjet"] },
    Airline { icao: "WZZ", name: "Wizz Air", also: &["WUK", "WAZ"], words: &["wizz"] },
];

pub fn by_code(code: &str) -> Option<&'static Airline> {
    let code = code.trim();
    AIRLINES
        .iter()
        .find(|a| a.icao.eq_ignore_ascii_case(code) || a.also.iter().any(|c| c.eq_ignore_ascii_case(code)))
}

/// Finds an airline in free text such as "Norwegian Air Shuttle (LN-NOV)" or
/// "FSLTL_B738_NAX". Brand words win over bare three-letter codes.
pub fn find_in(text: &str) -> Option<&'static Airline> {
    let lower = format!(" {} ", text.to_ascii_lowercase().replace(['_', '-', '(', ')', '|', '.', ','], " "));
    let squeezed = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    let padded = format!(" {squeezed} ");
    if let Some(a) = AIRLINES.iter().find(|a| a.words.iter().any(|w| padded.contains(&format!(" {} ", w.trim())))) {
        return Some(a);
    }
    squeezed.split(' ').filter(|t| t.len() == 3).find_map(by_code)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detected {
    pub icao: String,
    pub name: String,
    /// Where it came from, for the status line.
    pub source: &'static str,
}

/// Everything that can tell us the airline, strongest first.
pub struct Clues<'a> {
    pub simbrief_icao: &'a str,
    pub livery: &'a str,
    pub atc_airline: &'a str,
    pub title: &'a str,
}

pub fn detect(c: &Clues) -> Option<Detected> {
    let found = |a: &Airline, source| Detected { icao: a.icao.to_string(), name: a.name.to_string(), source };
    if !c.simbrief_icao.trim().is_empty() {
        return Some(match by_code(c.simbrief_icao) {
            Some(a) => found(a, "flight plan"),
            None => Detected { icao: c.simbrief_icao.trim().to_ascii_uppercase(), name: String::new(), source: "flight plan" },
        });
    }
    for (text, source) in [(c.livery, "livery"), (c.atc_airline, "livery callsign"), (c.title, "aircraft title")] {
        if let Some(a) = find_in(text) {
            return Some(found(a, source));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_norwegian_everywhere() {
        assert_eq!(find_in("Norwegian Air Shuttle (LN-NOV | 2019)").unwrap().icao, "NAX");
        assert_eq!(find_in("FSLTL_FAIB_B738_NSZ-Norwegian_Rednose").unwrap().icao, "NAX");
        assert_eq!(find_in("FSLTL_B738_NOZ_WHITE_TAIL").unwrap().icao, "NAX");
        assert_eq!(find_in("NORSHUTTLE").unwrap().icao, "NAX");
        assert_eq!(by_code("ibk").unwrap().name, "Norwegian");
    }

    #[test]
    fn avoids_false_matches() {
        assert!(find_in("PMDG 737-800 House").is_none());
        assert!(find_in("Unitedly painted").is_none());
        assert_eq!(find_in("United Airlines (N37267 | 2019 | EvoBlu)").unwrap().icao, "UAL");
    }

    #[test]
    fn flight_plan_beats_livery() {
        let c = Clues { simbrief_icao: "NAX", livery: "United Airlines", atc_airline: "", title: "" };
        assert_eq!(detect(&c).unwrap().source, "flight plan");
        let c = Clues { simbrief_icao: "", livery: "", atc_airline: "Norwegian", title: "PMDG 737-800" };
        assert_eq!(detect(&c).unwrap(), Detected { icao: "NAX".into(), name: "Norwegian".into(), source: "livery callsign" });
    }
}
