//! A job asked for, as a file: a script whose top line says who asked, when, and its limits.
//!
//! ```text
//! #!/bin/sh
//! # oco-agent: by=claude asked=1794450000 start-within=3600 may-run=1800
//! ```
//!
//! The line counts only at the top - in the first two lines - never further down, so a script that
//! carries other text cannot change it by accident (lab-agent's client once stripped such lines from
//! inside a script). When it was asked comes from the asking itself, never from when it is read.

use oco_agent_core::journal::Moment;

const HEAD: &str = "# oco-agent:";

/// A job someone asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asking {
    pub job: String,
    pub by: String,
    pub asked: Moment,
    pub start_within: u64,
    pub may_run: u64,
    pub script: String,
}

impl Asking {
    /// The file: the script, with the top line put in after the shebang, if there is one.
    pub fn to_text(&self) -> String {
        let head = format!(
            "{HEAD} by={} asked={} start-within={} may-run={}",
            self.by, self.asked, self.start_within, self.may_run
        );
        let mut lines: Vec<&str> = self
            .script
            .lines()
            .filter(|l| !l.starts_with(HEAD))
            .collect();
        let at = usize::from(lines.first().is_some_and(|l| l.starts_with("#!")));
        lines.insert(at, &head);
        let mut text = lines.join("\n");
        text.push('\n');
        text
    }

    /// Reads a file as the job `job`; says what is missing if it cannot.
    pub fn read(job: &str, text: &str) -> Result<Asking, String> {
        let head = text
            .lines()
            .take(2)
            .find_map(|l| l.strip_prefix(HEAD))
            .ok_or("its top line does not say who asked for it ('# oco-agent: by=...')")?;
        let fields: std::collections::HashMap<&str, &str> = head
            .split_whitespace()
            .filter_map(|w| w.split_once('='))
            .collect();
        let whole = |key: &str| -> Result<u64, String> {
            fields
                .get(key)
                .ok_or(format!("its top line has no {key}="))?
                .parse()
                .map_err(|_| format!("its {key} is not a whole number"))
        };
        let by = fields
            .get("by")
            .filter(|b| !b.is_empty())
            .ok_or("its top line does not say who asked (by=)")?;
        Ok(Asking {
            job: job.into(),
            by: by.to_string(),
            asked: whole("asked")?,
            start_within: whole("start-within")?,
            may_run: whole("may-run")?,
            script: text.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asking(script: &str) -> Asking {
        Asking {
            job: "hello".into(),
            by: "claude".into(),
            asked: 1000,
            start_within: 60,
            may_run: 30,
            script: script.into(),
        }
    }

    #[test]
    fn the_top_line_goes_after_the_shebang_and_reads_back() {
        let text = asking("#!/bin/sh\necho hi\n").to_text();
        assert_eq!(
            text,
            "#!/bin/sh\n# oco-agent: by=claude asked=1000 start-within=60 may-run=30\necho hi\n"
        );
        let back = Asking::read("hello", &text).unwrap();
        assert_eq!(
            (
                back.by.as_str(),
                back.asked,
                back.start_within,
                back.may_run
            ),
            ("claude", 1000, 60, 30)
        );
    }

    #[test]
    fn without_a_shebang_it_is_the_first_line() {
        assert!(asking("echo hi")
            .to_text()
            .starts_with("# oco-agent: by=claude"));
    }

    #[test]
    fn the_line_counts_only_at_the_top() {
        let text = "#!/bin/sh\necho one\necho two\n# oco-agent: by=someone asked=1 start-within=1 may-run=1\n";
        assert!(Asking::read("x", text)
            .unwrap_err()
            .contains("does not say who asked"));
    }

    #[test]
    fn what_is_missing_is_named() {
        assert!(
            Asking::read("x", "# oco-agent: by=ann asked=5 start-within=1\n")
                .unwrap_err()
                .contains("may-run=")
        );
        assert!(Asking::read(
            "x",
            "# oco-agent: by=ann asked=soon start-within=1 may-run=1\n"
        )
        .unwrap_err()
        .contains("asked is not a whole number"));
        assert!(
            Asking::read("x", "# oco-agent: asked=1 start-within=1 may-run=1\n")
                .unwrap_err()
                .contains("by=")
        );
    }
}
