//! Line diffs for previews (conversation fork compare, Azure DevOps PR diffs): Myers' O(ND)
//! algorithm on lines, printed as unified hunks with 3 lines of context.

use std::fmt::Write as _;

/// Lines per side above which no line diff is computed (a one-line note is printed instead).
const MAX_LINES: usize = 20_000;
/// Edit distance above which the diff gives up (huge rewrites are shown as replaced).
const MAX_D: usize = 4_000;
const CONTEXT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Keep,
    Del,
    Add,
}

/// Edit script from `a` to `b` (Myers 1986, greedy forward search with trace).
#[allow(clippy::many_single_char_names)] // short names of the math or the protocol
fn script(a: &[&str], b: &[&str]) -> Option<Vec<Op>> {
    let (n, m) = (a.len(), b.len());
    let max = (n + m).min(MAX_D);
    let off = max + 1;
    let mut v = vec![0usize; 2 * max + 3];
    let mut trace: Vec<Vec<usize>> = Vec::new();
    #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)] // bounded by MAX_LINES/MAX_D
    for d in 0..=max as isize {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let ki = (k + off as isize) as usize;
            let mut x = if k == -d || (k != d && v[ki - 1] < v[ki + 1]) {
                v[ki + 1]
            } else {
                v[ki - 1] + 1
            };
            let mut y = (x as isize - k) as usize;
            while x < n && y < m && a[x] == b[y] {
                x += 1;
                y += 1;
            }
            v[ki] = x;
            if x >= n && y >= m {
                return Some(backtrack(&trace, n, m, off));
            }
            k += 2;
        }
    }
    None
}

#[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)] // indices bounded by the search
#[allow(clippy::many_single_char_names)] // short names of the math or the protocol
fn backtrack(trace: &[Vec<usize>], n: usize, m: usize, off: usize) -> Vec<Op> {
    let (mut x, mut y) = (n as isize, m as isize);
    let mut ops = Vec::new();
    for d in (0..trace.len()).rev() {
        let v = &trace[d];
        let d = d as isize;
        let k = x - y;
        let ki = |k: isize| (k + off as isize) as usize;
        let prev_k = if k == -d || (k != d && v[ki(k - 1)] < v[ki(k + 1)]) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = v[ki(prev_k)] as isize;
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            ops.push(Op::Keep);
            x -= 1;
            y -= 1;
        }
        if d > 0 {
            ops.push(if x == prev_x { Op::Add } else { Op::Del });
        }
        x = prev_x;
        y = prev_y;
    }
    ops.reverse();
    ops
}

/// Unified diff of `old` → `new` under `a/<path>` / `b/<path>` headers; `None` means the file
/// does not exist on that side. Empty when both are equal.
#[allow(clippy::many_single_char_names)] // short names of the math or the protocol
pub fn unified(path: &str, old: Option<&str>, new: Option<&str>) -> String {
    if old == new {
        return String::new();
    }
    let from = if old.is_some() {
        format!("a/{path}")
    } else {
        "/dev/null".into()
    };
    let to = if new.is_some() {
        format!("b/{path}")
    } else {
        "/dev/null".into()
    };
    let mut out = format!("--- {from}\n+++ {to}\n");
    let a: Vec<&str> = old.unwrap_or_default().lines().collect();
    let b: Vec<&str> = new.unwrap_or_default().lines().collect();
    let ops = if a.len() > MAX_LINES || b.len() > MAX_LINES {
        None
    } else {
        script(&a, &b)
    };
    let Some(ops) = ops else {
        let _ = writeln!(
            out,
            "@@ -1,{} +1,{} @@ (too large for a line diff)",
            a.len(),
            b.len()
        );
        return out;
    };
    // Position of every op in a and b, then hunks around the changes.
    let mut pos = Vec::with_capacity(ops.len());
    let (mut i, mut j) = (0usize, 0usize);
    for op in &ops {
        pos.push((i, j));
        match op {
            Op::Keep => {
                i += 1;
                j += 1;
            }
            Op::Del => i += 1,
            Op::Add => j += 1,
        }
    }
    let changed: Vec<usize> = (0..ops.len()).filter(|&k| ops[k] != Op::Keep).collect();
    let mut k = 0;
    while k < changed.len() {
        let start = changed[k].saturating_sub(CONTEXT);
        let mut end = changed[k];
        while k < changed.len() && changed[k] <= end + 2 * CONTEXT + 1 {
            end = changed[k];
            k += 1;
        }
        let end = (end + CONTEXT + 1).min(ops.len());
        let (a0, b0) = pos[start];
        let a_len = ops[start..end].iter().filter(|o| **o != Op::Add).count();
        let b_len = ops[start..end].iter().filter(|o| **o != Op::Del).count();
        let _ = writeln!(
            out,
            "@@ -{},{a_len} +{},{b_len} @@",
            a0 + usize::from(a_len > 0),
            b0 + usize::from(b_len > 0)
        );
        for (op, (x, y)) in ops[start..end].iter().zip(&pos[start..end]) {
            let _ = match op {
                Op::Keep => writeln!(out, " {}", a[*x]),
                Op::Del => writeln!(out, "-{}", a[*x]),
                Op::Add => writeln!(out, "+{}", b[*y]),
            };
        }
    }
    out
}

/// `(added, removed)` line counts of a unified diff.
pub fn stats(diff: &str) -> (usize, usize) {
    diff.lines().fold((0, 0), |(a, r), l| {
        if l.starts_with('+') && !l.starts_with("+++") {
            (a + 1, r)
        } else if l.starts_with('-') && !l.starts_with("---") {
            (a, r + 1)
        } else {
            (a, r)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunks_with_context_and_counts() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\n";
        let new = "a\nb\nC\nd\ne\nf\ng\nh\ni\n";
        let d = unified("x.txt", Some(old), Some(new));
        assert!(d.starts_with("--- a/x.txt\n+++ b/x.txt\n@@ -1,"));
        assert!(d.contains("-c\n+C\n"));
        assert!(d.contains("+i\n"));
        assert_eq!(stats(&d), (2, 1));
        assert_eq!(unified("x", Some("same"), Some("same")), "");
    }

    #[test]
    fn new_and_deleted_files() {
        let d = unified("n.rs", None, Some("fn x() {}\n"));
        assert!(d.starts_with("--- /dev/null\n+++ b/n.rs\n@@ -0,0 +1,1 @@\n+fn x() {}"));
        let d = unified("n.rs", Some("a\nb\n"), None);
        assert!(d.contains("+++ /dev/null") && d.contains("-a\n-b\n"));
        assert_eq!(stats(&d), (0, 2));
    }

    #[test]
    fn far_apart_changes_make_two_hunks() {
        let old: String = (0..40).fold(String::new(), |mut s, i| {
            let _ = std::fmt::Write::write_fmt(&mut s, format_args!("l{i}\n"));
            s
        });
        let new = old.replace("l2\n", "X\n").replace("l35\n", "Y\n");
        let d = unified("f", Some(&old), Some(&new));
        assert_eq!(d.matches("@@ -").count(), 2);
        assert_eq!(stats(&d), (2, 2));
    }
}
