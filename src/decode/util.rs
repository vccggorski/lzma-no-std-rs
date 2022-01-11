use crate::io;

pub fn is_eof<R: io::BufRead>(input: &mut R) -> io::Result<bool> {
    let buf = input.fill_buf()?;
    Ok(buf.is_empty())
}

pub const fn exact_log2(mut value: usize) -> Option<usize> {
    if value == 0 {
        return None;
    }
    let mut result = 0;
    let mut bit_count = 0;
    while value != 0 {
        if (value & 1) == 1 {
            bit_count += 1;
            if bit_count >= 2 {
                return None;
            }
        }
        value >>= 1;
        result += 1;
    }
    Some(result - 1)
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn verify_exact_log2() {
        assert_eq!(Some(0), exact_log2(1 << 0));
        assert_eq!(Some(1), exact_log2(1 << 1));
        assert_eq!(None, exact_log2(3));
        assert_eq!(Some(2), exact_log2(1 << 2));
        assert_eq!(None, exact_log2(5));
        assert_eq!(None, exact_log2(7));
        assert_eq!(Some(3), exact_log2(1 << 3));
        assert_eq!(None, exact_log2(9));
        assert_eq!(None, exact_log2(255));
        assert_eq!(Some(8), exact_log2(1 << 8));
        assert_eq!(None, exact_log2(257));
        assert_eq!(None, exact_log2((1 << 31) - 1));
        assert_eq!(Some(31), exact_log2(1 << 31));
        assert_eq!(None, exact_log2((1 << 31) + 1));
    }
}

