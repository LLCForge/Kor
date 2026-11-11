//! Minimal decoder for the pprof `Profile` protobuf (github.com/google/pprof, profile.proto):
//! sample types, samples, locations (with inlined lines) and functions. Labels, mappings and
//! addresses are not needed for collapsed stacks and are skipped.

use std::collections::HashMap;

use super::{Profile, Unit};

enum Val<'a> {
    Int(u64),
    Bytes(&'a [u8]),
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, i: 0 }
    }

    fn varint(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *self.b.get(self.i)?;
            self.i += 1;
            v |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }

    /// Next field, or `None` at the end; malformed input ends the message.
    fn field(&mut self) -> Option<(u64, Val<'a>)> {
        if self.i >= self.b.len() {
            return None;
        }
        let key = self.varint()?;
        let val = match key & 7 {
            0 => Val::Int(self.varint()?),
            1 => {
                self.i = self.i.checked_add(8).filter(|&e| e <= self.b.len())?;
                Val::Int(0)
            }
            2 => {
                let len = usize::try_from(self.varint()?).ok()?;
                let end = self.i.checked_add(len).filter(|&e| e <= self.b.len())?;
                let s = &self.b[self.i..end];
                self.i = end;
                Val::Bytes(s)
            }
            5 => {
                self.i = self.i.checked_add(4).filter(|&e| e <= self.b.len())?;
                Val::Int(0)
            }
            _ => return None,
        };
        Some((key >> 3, val))
    }
}

/// Repeated integer field, packed or not.
fn ints(v: &Val<'_>, out: &mut Vec<u64>) {
    match v {
        Val::Int(n) => out.push(*n),
        Val::Bytes(b) => {
            let mut r = Reader::new(b);
            while let Some(n) = r.varint() {
                out.push(n);
            }
        }
    }
}

fn int(v: &Val<'_>) -> u64 {
    match v {
        Val::Int(n) => *n,
        Val::Bytes(_) => 0,
    }
}

#[derive(Default)]
struct Func {
    name: u64,
    file: u64,
    start: u64,
}

/// Decode a (gunzipped) pprof profile. Returns the stacks and whether it is a memory profile
/// (`alloc_space` weights, `alloc_objects` as the count) rather than CPU (`samples`, else
/// the first value).
#[allow(clippy::too_many_lines)] // one pass over the message fields
pub(super) fn parse(b: &[u8]) -> Result<(Profile, bool), String> {
    let mut strings: Vec<String> = Vec::new();
    let mut types: Vec<(u64, u64)> = Vec::new();
    let mut samples: Vec<(Vec<u64>, Vec<u64>)> = Vec::new();
    let mut locations: HashMap<u64, Vec<(u64, u64)>> = HashMap::new();
    let mut funcs: HashMap<u64, Func> = HashMap::new();
    let mut r = Reader::new(b);
    while let Some((f, v)) = r.field() {
        let Val::Bytes(m) = v else { continue };
        let mut sub = Reader::new(m);
        match f {
            1 => {
                let (mut t, mut u) = (0, 0);
                while let Some((g, x)) = sub.field() {
                    match g {
                        1 => t = int(&x),
                        2 => u = int(&x),
                        _ => {}
                    }
                }
                types.push((t, u));
            }
            2 => {
                let (mut locs, mut vals) = (Vec::new(), Vec::new());
                while let Some((g, x)) = sub.field() {
                    match g {
                        1 => ints(&x, &mut locs),
                        2 => ints(&x, &mut vals),
                        _ => {}
                    }
                }
                samples.push((locs, vals));
            }
            4 => {
                let (mut id, mut lines) = (0, Vec::new());
                while let Some((g, x)) = sub.field() {
                    match (g, x) {
                        (1, x) => id = int(&x),
                        (4, Val::Bytes(l)) => {
                            let (mut func, mut line) = (0, 0);
                            let mut lr = Reader::new(l);
                            while let Some((h, y)) = lr.field() {
                                match h {
                                    1 => func = int(&y),
                                    2 => line = int(&y),
                                    _ => {}
                                }
                            }
                            lines.push((func, line));
                        }
                        _ => {}
                    }
                }
                locations.insert(id, lines);
            }
            5 => {
                let (mut id, mut fu) = (0, Func::default());
                while let Some((g, x)) = sub.field() {
                    match g {
                        1 => id = int(&x),
                        2 => fu.name = int(&x),
                        4 => fu.file = int(&x),
                        5 => fu.start = int(&x),
                        _ => {}
                    }
                }
                funcs.insert(id, fu);
            }
            6 => strings.push(String::from_utf8_lossy(m).into_owned()),
            _ => {}
        }
    }
    if types.is_empty() && samples.is_empty() {
        return Err("pprof: no sample types or samples".into());
    }
    let s = |i: u64| {
        usize::try_from(i)
            .ok()
            .and_then(|i| strings.get(i))
            .map_or("", String::as_str)
    };
    let idx = |name: &str| types.iter().position(|t| s(t.0) == name);
    let (memory, value, count) = match idx("alloc_space").or_else(|| idx("inuse_space")) {
        Some(space) => (
            true,
            space,
            idx("alloc_objects").or_else(|| idx("inuse_objects")),
        ),
        None => (false, idx("samples").unwrap_or(0), None),
    };
    let unit = if memory {
        Unit::Bytes
    } else if idx("samples") == Some(value) {
        Unit::Samples
    } else if types.get(value).is_some_and(|t| s(t.1) == "nanoseconds") {
        Unit::Nanoseconds
    } else {
        Unit::Samples
    };
    let mut p = Profile::new(unit);
    let mut total_count = 0u64;
    for (locs, vals) in &samples {
        let w = vals.get(value).copied().unwrap_or(0);
        if let Some(c) = count {
            total_count += vals.get(c).copied().unwrap_or(0);
        }
        // Locations are leaf first; a location's lines are innermost first.
        let mut stack: Vec<u32> = Vec::new();
        for l in locs {
            for &(fid, line) in locations.get(l).map_or(&[][..], Vec::as_slice) {
                let Some(fu) = funcs.get(&fid) else { continue };
                let line = if fu.start > 0 { fu.start } else { line };
                let f = p.frame(
                    s(fu.name),
                    Some(s(fu.file)),
                    u32::try_from(line).ok().filter(|l| *l > 0),
                );
                stack.push(f);
            }
        }
        stack.reverse();
        p.add(stack, w);
    }
    if count.is_some() {
        p.count = Some(total_count);
    }
    Ok((p, memory))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        assert!(parse(&[]).is_err());
        assert!(parse(&[0x0a, 0xff, 0xff]).is_err());
        assert!(parse(&[0xff; 32]).is_err());
    }
}
