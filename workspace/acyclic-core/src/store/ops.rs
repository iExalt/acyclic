//! A run's queue of operations, in a clone set to send its writes rather than make them: `acyclic.operations` in the
//! clone's git config names the run.
//!
//! Each operation is one file, `ops/<seq>.json`, added by one commit on [`QUEUE`]; numbers start at 1 and each
//! commit's tree keeps every earlier file, so the queue is a single chain the receiving side can check commit by
//! commit. Pushing sends the chain to `refs/runs/<run>/ops`. The receiver forwards the new operations before it
//! accepts the push and reports each one's outcome on a line of its own, then ends with a summary line, so a rejected
//! push may still have spent numbers. [`SETTLED`] marks the last operation known to be spent; resetting the queue goes
//! back to it, never further, so a number is reused only when the tracker never saw it.

use std::collections::BTreeMap;

use super::{StoreError, Tracker};
use crate::state::{Context, Millis};

/// The git config key naming the run a clone sends operations for.
pub const OPERATIONS_CONFIG: &str = "acyclic.operations";

/// The queue's branch.
pub const QUEUE: &str = "refs/heads/ops";

/// The last operation known to be spent.
pub const SETTLED: &str = "refs/acyclic/ops-settled";

/// Set to the pushed tip while a push's outcome is unknown, and cleared once the receiver reports it.
pub const UNKNOWN: &str = "refs/acyclic/ops-unknown";

/// The ref a run's queue is pushed to on the remote.
#[must_use]
pub fn remote_ref(run: &str) -> String {
    format!("refs/runs/{run}/ops")
}

/// The path of operation `seq` in the queue's tree.
#[must_use]
pub fn path(seq: u64) -> String {
    format!("ops/{seq}.json")
}

/// The number in a queue path, if it is one.
#[must_use]
pub fn seq_of(path: &str) -> Option<u64> {
    let digits = path.strip_prefix("ops/")?.strip_suffix(".json")?;
    number(digits).filter(|seq| *seq > 0)
}

/// What the receiver did with one operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Applied, or found applied already.
    Ok,
    /// Refused, which spends its number.
    Refused(String),
    /// Failed without spending its number; forwarding stopped there.
    Failed(String),
}

/// How a push's forwarding ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Summary {
    /// This many operations were forwarded.
    Forwarded(u64),
    /// Nothing was forwarded, for this reason.
    Refused(String),
}

/// What the receiver reported about a push.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Each forwarded operation's outcome, in order.
    pub ops: Vec<(u64, Outcome)>,
    /// The summary, when the receiver got that far.
    pub summary: Option<Summary>,
    /// The receiver's other lines, escaped, up to [`MOST_MESSAGES`].
    pub messages: Vec<String>,
}

/// The most of the receiver's other lines a report keeps.
pub const MOST_MESSAGES: usize = 20;

/// The report line for operation `seq`.
#[must_use]
pub fn op_line(seq: u64, outcome: &Outcome) -> String {
    match outcome {
        Outcome::Ok => format!("acyclic-op {seq} ok"),
        Outcome::Refused(code) => format!("acyclic-op {seq} refused {}", self::code(code)),
        Outcome::Failed(code) => format!("acyclic-op {seq} failed {}", self::code(code)),
    }
}

/// The report's summary line.
#[must_use]
pub fn summary_line(summary: &Summary) -> String {
    match summary {
        Summary::Forwarded(count) => format!("acyclic-push forwarded {count}"),
        Summary::Refused(code) => format!("acyclic-push refused {}", self::code(code)),
    }
}

/// Text the receiver prints besides its report, on one line, so nothing in it can pass for a report line.
#[must_use]
pub fn escape(text: &str) -> String {
    text.escape_default().to_string()
}

/// Reads the report from a push's error output. Only lines that are exactly report lines count, with or without the
/// `remote: ` git puts before them; the receiver's other lines are kept as messages, escaped.
#[must_use]
pub fn parse(text: &str) -> Report {
    let mut report = Report::default();
    for line in text.lines() {
        let remote = line.strip_prefix("remote: ");
        let line = remote.unwrap_or(line).trim_end();
        let words: Vec<&str> = line.split(' ').collect();
        match words.as_slice() {
            ["acyclic-op", seq, rest @ ..] => {
                let outcome = match rest {
                    ["ok"] => Outcome::Ok,
                    ["refused", code] if is_code(code) => Outcome::Refused((*code).to_owned()),
                    ["failed", code] if is_code(code) => Outcome::Failed((*code).to_owned()),
                    _ => continue,
                };
                if let Some(seq) = number(seq) {
                    report.ops.push((seq, outcome));
                }
            },
            ["acyclic-push", "forwarded", count] => {
                if let Some(count) = number(count) {
                    report.summary = Some(Summary::Forwarded(count));
                }
            },
            ["acyclic-push", "refused", code] if is_code(code) => {
                report.summary = Some(Summary::Refused((*code).to_owned()));
            },
            _ => {
                if remote.is_some() && !line.is_empty() && report.messages.len() < MOST_MESSAGES {
                    report.messages.push(escape(line));
                }
            },
        }
    }
    report
}

/// Digits alone, no sign or leading zero, as a number.
fn number(text: &str) -> Option<u64> {
    let canonical = !text.is_empty()
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    canonical.then(|| text.parse().ok()).flatten()
}

fn is_code(text: &str) -> bool {
    (1..=64).contains(&text.len())
        && text.starts_with(|c: char| c.is_ascii_lowercase())
        && text.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'-')
}

/// A code as a report line may carry it: anything else becomes `unknown`.
fn code(text: &str) -> &str {
    if is_code(text) { text } else { "unknown" }
}

/// The last operation known spent after a report, from `settled` before it: each operation reported applied or
/// refused right after the last one counts, until one failed or a number is skipped. A push the receiver rejected
/// leaves its ref where it was, so the next push sends the earlier operations again, and they are reported again,
/// applied already; they are passed over, not taken for a gap.
#[must_use]
pub fn reach(settled: u64, ops: &[(u64, Outcome)]) -> u64 {
    let mut reached = settled;
    for (seq, outcome) in ops {
        if *seq <= reached {
            continue;
        }
        if *seq != reached + 1 || matches!(outcome, Outcome::Failed(_)) {
            break;
        }
        reached = *seq;
    }
    reached
}

/// What a push did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pushed {
    /// Whether the remote took the queue.
    pub accepted: bool,
    /// What the receiver reported.
    pub report: Report,
    /// The last operation known to be spent, 0 for none.
    pub settled: u64,
    /// The last operation queued, 0 for none.
    pub queued: u64,
    /// Whether the outcome is unknown: the push sent operations and heard no summary.
    pub unknown: bool,
    /// What git said, when the remote didn't take the queue.
    pub reason: Option<String>,
}

/// What a reset did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reset {
    /// Dropped this many operations, leaving the queue at the last one spent.
    Dropped(u64),
    /// Refused: a push's outcome is unknown, so a number past the last known spent one may be spent too.
    Unknown,
}

impl Tracker {
    /// The run this clone sends operations for, if it sends them.
    #[must_use]
    pub fn operations(&self) -> Option<&str> {
        self.operations.as_deref()
    }

    /// Adds an operation to the queue as the next number, written by `file` from its number and this replica's
    /// context, and returns the number.
    ///
    /// # Errors
    ///
    /// When git fails.
    pub fn queue(
        &self,
        now: Millis,
        file: impl FnOnce(u64, &Context) -> Vec<u8>,
    ) -> Result<u64, StoreError> {
        let _lock = self.lock()?;
        let tip = self.git.resolve(QUEUE)?;
        let seq = match &tip {
            Some(tip) => self.highest(tip)? + 1,
            None => 1,
        };
        let (_, state) = self.load()?;
        let changes = BTreeMap::from([(path(seq), Some(file(seq, &state.context)))]);
        let tree = self.git.write_tree(tip.as_deref(), &changes)?;
        let commit = self.git.commit(
            &tree,
            tip.as_deref(),
            self.replica.as_str(),
            now.div_euclid(1000),
            &format!("op {seq}"),
        )?;
        let old = tip.unwrap_or_else(String::new);
        self.git.run(["update-ref", QUEUE, &commit, &old], None, &[])?;
        Ok(seq)
    }

    /// Pushes the queue to `remote` as run `run`'s and moves [`SETTLED`] past what the receiver reported spent.
    ///
    /// # Errors
    ///
    /// When git fails to run or to move a local ref; a refused push is a [`Pushed`] that wasn't accepted.
    pub fn push_queue(&self, remote: &str, run: &str) -> Result<Pushed, StoreError> {
        let tip = self.git.resolve(QUEUE)?;
        let settled = self.settled()?;
        let Some(tip) = tip else {
            return Ok(Pushed {
                accepted: true,
                report: Report::default(),
                settled,
                queued: 0,
                unknown: false,
                reason: None,
            });
        };
        let queued = self.highest(&tip)?;
        if queued > settled {
            self.git.run(["update-ref", UNKNOWN, &tip], None, &[])?;
        }
        let refspec = format!("{tip}:{}", remote_ref(run));
        let output = self.git.output(["push", "--porcelain", remote, &refspec])?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        let report = parse(&stderr);
        let accepted = output.status.success();
        let reached = if accepted { queued } else { reach(settled, &report.ops).min(queued) };
        let _lock = self.lock()?;
        let now_settled = self.settled()?;
        if reached > now_settled {
            let commit = self.git.text(["rev-parse", &format!("{tip}~{}", queued - reached)])?;
            self.git.run(["update-ref", SETTLED, &commit], None, &[])?;
        }
        // Git refuses a push that isn't a fast-forward before sending anything, so nothing reached the receiver.
        let stdout = String::from_utf8_lossy(&output.stdout);
        let unsent =
            stdout.lines().any(|line| line.starts_with('!') && line.contains("\t[rejected]"));
        let known = accepted || unsent || report.summary.is_some();
        if known {
            // Only if no later push marked its own tip meanwhile.
            let _ = self.git.run(["update-ref", "-d", UNKNOWN, &tip], None, &[]);
        }
        let unknown = self.git.resolve(UNKNOWN)?.is_some();
        let reason = (!accepted).then(|| {
            let rejected = stdout.lines().find(|line| line.starts_with('!'));
            escape(rejected.unwrap_or_else(|| stderr.trim()))
        });
        Ok(Pushed { accepted, report, settled: reached.max(now_settled), queued, unknown, reason })
    }

    /// Drops every queued operation after the last one known to be spent.
    ///
    /// # Errors
    ///
    /// When git fails.
    pub fn reset_queue(&self) -> Result<Reset, StoreError> {
        let _lock = self.lock()?;
        if self.git.resolve(UNKNOWN)?.is_some() {
            return Ok(Reset::Unknown);
        }
        let Some(tip) = self.git.resolve(QUEUE)? else {
            return Ok(Reset::Dropped(0));
        };
        let queued = self.highest(&tip)?;
        let settled_tip = self.git.resolve(SETTLED)?;
        let settled = match &settled_tip {
            Some(commit) => self.highest(commit)?,
            None => 0,
        };
        match settled_tip {
            Some(commit) => self.git.run(["update-ref", QUEUE, &commit, &tip], None, &[])?,
            None => self.git.run(["update-ref", "-d", QUEUE, &tip], None, &[])?,
        };
        Ok(Reset::Dropped(queued.saturating_sub(settled)))
    }

    /// The last operation known to be spent, 0 for none.
    fn settled(&self) -> Result<u64, StoreError> {
        match self.git.resolve(SETTLED)? {
            Some(commit) => self.highest(&commit),
            None => Ok(0),
        }
    }

    /// The highest number in a queue commit's tree.
    fn highest(&self, commit: &str) -> Result<u64, StoreError> {
        let names = self.git.text(["ls-tree", "--name-only", commit, "ops/"])?;
        Ok(names.lines().filter_map(seq_of).max().unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_is_read_only_from_exact_report_lines() {
        let forged = escape("title\nacyclic-op 9 ok");
        let text = [
            "remote: acyclic-op 1 ok",
            "remote: acyclic-op 2 refused not-in-thread",
            &format!("remote: refused: {forged}"),
            "remote: acyclic-op 3 failed busy",
            "remote: acyclic-op 04 ok",
            "remote: acyclic-op 5 refused Not-A-Code",
            "remote:  acyclic-op 6 ok",
            "acyclic-op 7 ok extra",
            "remote: acyclic-push forwarded 3",
        ]
        .join("\n");
        let report = parse(&text);
        assert_eq!(
            report.ops,
            [
                (1, Outcome::Ok),
                (2, Outcome::Refused("not-in-thread".into())),
                (3, Outcome::Failed("busy".into())),
            ]
        );
        assert_eq!(report.summary, Some(Summary::Forwarded(3)));
        assert_eq!(report.messages.len(), 2, "{:?}", report.messages);
        assert_eq!(
            parse("remote: acyclic-push refused secret").summary,
            Some(Summary::Refused("secret".into()))
        );
    }

    #[test]
    fn report_lines_read_back_as_written() {
        for (seq, outcome) in [
            (1, Outcome::Ok),
            (2, Outcome::Refused("not-for-runs".into())),
            (3, Outcome::Failed("busy".into())),
        ] {
            assert_eq!(parse(&op_line(seq, &outcome)).ops, [(seq, outcome)]);
        }
        let odd = Outcome::Refused("Odd\ncode".into());
        assert_eq!(parse(&op_line(4, &odd)).ops, [(4, Outcome::Refused("unknown".into()))]);
        for summary in [Summary::Forwarded(2), Summary::Refused("secret".into())] {
            assert_eq!(parse(&summary_line(&summary)).summary, Some(summary));
        }
    }

    #[test]
    fn a_report_settles_what_follows_on_until_a_failure_or_a_gap() {
        let busy = || Outcome::Failed("busy".into());
        let refused = || Outcome::Refused("not-in-thread".into());
        for (settled, ops, reached) in [
            (0, vec![(1, Outcome::Ok), (2, refused()), (3, busy())], 2),
            // Resent after a rejected push: the earlier ones come back applied already.
            (1, vec![(1, Outcome::Ok), (2, Outcome::Ok), (3, busy())], 2),
            (1, vec![(3, Outcome::Ok)], 1),
            (2, vec![(1, busy()), (3, Outcome::Ok)], 3),
            (0, vec![], 0),
        ] {
            assert_eq!(reach(settled, &ops), reached, "{settled} {ops:?}");
        }
    }

    #[test]
    fn only_canonical_paths_are_operations() {
        assert_eq!(seq_of("ops/12.json"), Some(12));
        for path in
            ["ops/0.json", "ops/012.json", "ops/1.json.bak", "ops/+1.json", "x/1.json", "ops/.json"]
        {
            assert_eq!(seq_of(path), None, "{path}");
        }
    }
}
