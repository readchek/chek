use std::cmp::Ordering;

/// Compares names the way people number pages and volumes, so `page2` sorts
/// before `page10`. Letters compare case-insensitively.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    loop {
        match (a.first(), b.first()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (na, ra) = split_digits(a);
                let (nb, rb) = split_digits(b);
                let order = compare_digits(na, nb);
                if order != Ordering::Equal {
                    return order;
                }
                (a, b) = (ra, rb);
            }
            (Some(x), Some(y)) => {
                let order = x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                (a, b) = (&a[1..], &b[1..]);
            }
        }
    }
}

/// A key that sorts as [`natural_cmp`] does when compared byte by byte, for
/// storing in a database: lowercase, with every digit run padded to the same
/// width. Runs longer than that width keep their digits and may sort out of
/// place, which no real issue or volume number reaches.
pub fn natural_key(s: &str) -> String {
    const WIDTH: usize = 20;
    let mut key = String::with_capacity(s.len() + WIDTH);
    let mut rest = s;
    while let Some(start) = rest.find(|c: char| c.is_ascii_digit()) {
        key.push_str(&rest[..start].to_lowercase());
        let digits = &rest[start..];
        let end = digits
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(digits.len());
        let run = digits[..end].trim_start_matches('0');
        for _ in run.len()..WIDTH {
            key.push('0');
        }
        key.push_str(run);
        rest = &digits[end..];
    }
    key.push_str(&rest.to_lowercase());
    key
}

/// The number a label starts with, ignoring a leading word such as "Vol." or
/// "#": `"12"` → 12, `"Vol. 2"` → 2, `"1.5"` → 1.5.
pub fn leading_number(label: &str) -> Option<f64> {
    let start = label.find(|c: char| c.is_ascii_digit())?;
    let digits = &label[start..];
    let end = digits
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(digits.len());
    digits[..end].trim_end_matches('.').parse().ok()
}

fn split_digits(s: &[u8]) -> (&[u8], &[u8]) {
    let end = s
        .iter()
        .position(|c| !c.is_ascii_digit())
        .unwrap_or(s.len());
    s.split_at(end)
}

/// Compares digit runs by value without parsing, so long runs can't overflow.
fn compare_digits(a: &[u8], b: &[u8]) -> Ordering {
    let trim = |s: &[u8]| -> usize { s.iter().position(|&c| c != b'0').unwrap_or(s.len()) };
    let (a, b) = (&a[trim(a)..], &b[trim(b)..]);
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}
