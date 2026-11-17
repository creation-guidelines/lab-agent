//! lab-agent guide: how to work in the open-commons ecosystem - the practices for agents and
//! people, read live from grenudi/ideal-agent's database (a built-in copy when it cannot be reached), so an
//! improvement made once reaches every agent.

use anyhow::Result;
use std::time::Duration;

const LIVE: &str = "https://raw.githubusercontent.com/grenudi/ideal-agent/main/data/practices.json";
const BUILT_IN: &str = include_str!("../guide/practices.json");

pub fn run() -> Result<()> {
    let live = ureq::get(LIVE)
        .timeout(Duration::from_secs(10))
        .call()
        .ok()
        .and_then(|r| r.into_string().ok());
    let (text, source) = match live {
        Some(t) => (t, "live from ideal-agent"),
        None => (
            BUILT_IN.to_string(),
            "the built-in copy - ideal-agent could not be reached",
        ),
    };
    print!("{}", render(&text, source));
    Ok(())
}

/// The practices, numbered, each with why and for whom; lines that are not practices are skipped.
pub fn render(text: &str, source: &str) -> String {
    let mut out = format!("How to work in the open-commons ecosystem ({source})\n\n");
    // in the order of the ids, each numbered by the number its id begins with - as ideal-agent's session
    // start numbers them - so a practice is called by one number wherever it stands in the file
    let mut all: Vec<serde_json::Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|v| v["practice"].is_string())
        .collect();
    all.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    for (i, v) in all.iter().enumerate() {
        let practice = v["practice"].as_str().unwrap_or("");
        let n = v["id"]
            .as_str()
            .and_then(|id| id.split('-').next())
            .and_then(|d| d.parse::<u32>().ok())
            .map_or(i + 1, |n| n as usize);
        out.push_str(&format!(
            "{n}. {practice}\n   why: {}\n   for: {}\n\n",
            v["why"].as_str().unwrap_or(""),
            v["for"].as_str().unwrap_or("")
        ));
    }
    out.push_str("Keep your memory and preferences in line with these. When you find a better way, carry it back:\nhttps://github.com/grenudi/ideal-agent (data/practices.json, entered with tad r practice)\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn practices_are_numbered_with_their_reasons() {
        let t = "{\"id\":\"a\",\"practice\":\"Record it.\",\"why\":\"Memory fails.\",\"for\":\"agents\"}\nnot json\n{\"id\":\"b\",\"practice\":\"Land it.\",\"why\":\"Checks.\",\"for\":\"people\"}\n";
        let r = render(t, "test");
        assert!(r.starts_with("How to work in the open-commons ecosystem (test)"));
        assert!(r.contains("1. Record it.\n   why: Memory fails.\n   for: agents"));
        assert!(
            r.contains("2. Land it."),
            "the bad line is skipped, the numbering goes on"
        );
    }

    #[test]
    fn the_built_in_copy_holds_practices() {
        assert!(render(BUILT_IN, "built-in").contains("1. "));
    }
}
