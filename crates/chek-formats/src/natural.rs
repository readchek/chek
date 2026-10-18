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
