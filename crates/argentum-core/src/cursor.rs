//! URL-safe encoding for Toasty pagination cursors.
//!
//! Toasty's cursor-based pagination hands back opaque [`stmt::Value`]s
//! (`Page::next_cursor` / `Page::prev_cursor`) that `.after()` / `.before()`
//! accept to resume the walk. Server-rendered pagination needs those cursors
//! in a URL query parameter, and Toasty does not provide a string round-trip,
//! so this module encodes the value into a self-describing byte payload and
//! hex-encodes that into an ASCII-safe token (no percent-encoding or escaping
//! ambiguity in hrefs).
//!
//! The encoding preserves the exact [`Value`] variant (an `I64` decodes as an
//! `I64`, a `Uuid` as a `Uuid`), which matters because the engine compares the
//! cursor against the ordering column's typed value. Unsupported variants
//! (records nested inside fields, lists, objects, decimals) return an error
//! rather than silently degrading — an unsortable column has no business in a
//! cursor anyway.

use toasty_core::stmt::Value;
use topcoat::Result;

/// Version tag byte — bump on an incompatible layout change.
const VERSION: u8 = 1;

// Field tags. Single characters keep payloads compact.
const TAG_NULL: u8 = b'n';
const TAG_BOOL: u8 = b'b';
const TAG_I8: u8 = b'1';
const TAG_I16: u8 = b'2';
const TAG_I32: u8 = b'4';
const TAG_I64: u8 = b'8';
const TAG_U8: u8 = b'A';
const TAG_U16: u8 = b'B';
const TAG_U32: u8 = b'C';
const TAG_U64: u8 = b'D';
const TAG_F32: u8 = b'f';
const TAG_F64: u8 = b'g';
const TAG_STRING: u8 = b's';
const TAG_UUID: u8 = b'u';
const TAG_TIMESTAMP: u8 = b't';
const TAG_DATE: u8 = b'd';
const TAG_DATETIME: u8 = b'm';
const TAG_TIME: u8 = b'i';
const TAG_BYTES: u8 = b'x';
const TAG_RECORD: u8 = b'r';

/// Encode a cursor value into a URL-safe token (`[0-9a-f]` only).
///
/// # Errors
///
/// Errors when the cursor contains a variant this codec does not support.
pub fn encode(value: &Value) -> Result<String> {
    let mut payload = vec![VERSION];
    write_value(value, &mut payload)?;
    Ok(hex_encode(&payload))
}

/// Decode a token produced by [`encode`] back into a cursor [`Value`].
///
/// # Errors
///
/// Errors on malformed input (wrong length, unknown tag or version) so a
/// tampered or truncated `?after=`/`?before=` parameter fails loudly instead
/// of silently restarting pagination.
pub fn decode(token: &str) -> Result<Value> {
    let payload = hex_decode(token)?;
    let mut buf = &payload[..];
    let version = take(&mut buf, 1)?[0];
    if version != VERSION {
        return Err(std::io::Error::other(format!("cursor: unsupported version {version}")).into());
    }
    let (value, rest) = read_value(buf)?;
    if !rest.is_empty() {
        return Err(std::io::Error::other("cursor: trailing bytes after value").into());
    }
    Ok(value)
}

fn write_value(value: &Value, out: &mut Vec<u8>) -> Result<()> {
    match value {
        Value::Null => out.push(TAG_NULL),
        Value::Bool(b) => {
            out.push(TAG_BOOL);
            out.push(u8::from(*b));
        }
        Value::I8(v) => {
            out.push(TAG_I8);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Value::I16(v) => {
            out.push(TAG_I16);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Value::I32(v) => {
            out.push(TAG_I32);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Value::I64(v) => {
            out.push(TAG_I64);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Value::U8(v) => {
            out.push(TAG_U8);
            out.push(*v);
        }
        Value::U16(v) => {
            out.push(TAG_U16);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Value::U32(v) => {
            out.push(TAG_U32);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Value::U64(v) => {
            out.push(TAG_U64);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Value::F32(v) => {
            out.push(TAG_F32);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Value::F64(v) => {
            out.push(TAG_F64);
            out.extend_from_slice(&v.to_le_bytes());
        }
        Value::String(v) => {
            out.push(TAG_STRING);
            write_len_prefixed(v.as_bytes(), out);
        }
        Value::Uuid(v) => {
            out.push(TAG_UUID);
            out.extend_from_slice(v.as_bytes());
        }
        Value::Bytes(v) => {
            out.push(TAG_BYTES);
            write_len_prefixed(v, out);
        }
        Value::Timestamp(v) => {
            out.push(TAG_TIMESTAMP);
            write_len_prefixed(v.to_string().as_bytes(), out);
        }
        Value::Date(v) => {
            out.push(TAG_DATE);
            write_len_prefixed(v.to_string().as_bytes(), out);
        }
        Value::DateTime(v) => {
            out.push(TAG_DATETIME);
            write_len_prefixed(v.to_string().as_bytes(), out);
        }
        Value::Time(v) => {
            out.push(TAG_TIME);
            write_len_prefixed(v.to_string().as_bytes(), out);
        }
        Value::Record(record) => {
            out.push(TAG_RECORD);
            out.extend_from_slice(
                &u32::try_from(record.fields.len())
                    .map_err(|e| std::io::Error::other(format!("cursor: record too long: {e}")))?
                    .to_le_bytes(),
            );
            for field in &record.fields {
                write_value(field, out)?;
            }
        }
        other => {
            return Err(
                std::io::Error::other(format!("cursor: unsupported value {other:?}")).into(),
            );
        }
    }
    Ok(())
}

/// Reads one tagged value; returns it plus the remaining buffer.
fn read_value(buf: &[u8]) -> Result<(Value, &[u8])> {
    let mut buf = buf;
    let tag = take(&mut buf, 1)?[0];
    match tag {
        TAG_NULL => Ok((Value::Null, buf)),
        TAG_BOOL => {
            let b = take(&mut buf, 1)?[0];
            match b {
                0 => Ok((Value::Bool(false), buf)),
                1 => Ok((Value::Bool(true), buf)),
                _ => Err(std::io::Error::other("cursor: invalid bool byte").into()),
            }
        }
        TAG_I8 => {
            let bytes = take(&mut buf, 1)?;
            Ok((Value::I8(i8::from_le_bytes(bytes.try_into().unwrap())), buf))
        }
        TAG_I16 => Ok((
            Value::I16(i16::from_le_bytes(take(&mut buf, 2)?.try_into().unwrap())),
            buf,
        )),
        TAG_I32 => Ok((
            Value::I32(i32::from_le_bytes(take(&mut buf, 4)?.try_into().unwrap())),
            buf,
        )),
        TAG_I64 => Ok((
            Value::I64(i64::from_le_bytes(take(&mut buf, 8)?.try_into().unwrap())),
            buf,
        )),
        TAG_U8 => Ok((Value::U8(take(&mut buf, 1)?[0]), buf)),
        TAG_U16 => Ok((
            Value::U16(u16::from_le_bytes(take(&mut buf, 2)?.try_into().unwrap())),
            buf,
        )),
        TAG_U32 => Ok((
            Value::U32(u32::from_le_bytes(take(&mut buf, 4)?.try_into().unwrap())),
            buf,
        )),
        TAG_U64 => Ok((
            Value::U64(u64::from_le_bytes(take(&mut buf, 8)?.try_into().unwrap())),
            buf,
        )),
        TAG_F32 => Ok((
            Value::F32(f32::from_le_bytes(take(&mut buf, 4)?.try_into().unwrap())),
            buf,
        )),
        TAG_F64 => Ok((
            Value::F64(f64::from_le_bytes(take(&mut buf, 8)?.try_into().unwrap())),
            buf,
        )),
        TAG_STRING => {
            let s = read_len_prefixed(&mut buf)?;
            Ok((
                Value::String(String::from_utf8(s).map_err(|e| {
                    std::io::Error::other(format!("cursor: invalid utf-8 string: {e}"))
                })?),
                buf,
            ))
        }
        TAG_UUID => {
            let bytes = take(&mut buf, 16)?;
            Ok((
                Value::Uuid(
                    uuid::Uuid::from_slice(bytes)
                        .map_err(|e| std::io::Error::other(format!("cursor: invalid uuid: {e}")))?,
                ),
                buf,
            ))
        }
        TAG_TIMESTAMP => {
            let s = read_len_prefixed(&mut buf)?;
            let text = std::str::from_utf8(&s)
                .map_err(|e| std::io::Error::other(format!("cursor: invalid timestamp: {e}")))?;
            Ok((
                Value::Timestamp(text.parse::<jiff::Timestamp>().map_err(|e| {
                    std::io::Error::other(format!("cursor: invalid timestamp: {e}"))
                })?),
                buf,
            ))
        }
        TAG_DATE => {
            let s = read_len_prefixed(&mut buf)?;
            let text = std::str::from_utf8(&s)
                .map_err(|e| std::io::Error::other(format!("cursor: invalid date: {e}")))?;
            Ok((
                Value::Date(
                    text.parse::<jiff::civil::Date>()
                        .map_err(|e| std::io::Error::other(format!("cursor: invalid date: {e}")))?,
                ),
                buf,
            ))
        }
        TAG_DATETIME => {
            let s = read_len_prefixed(&mut buf)?;
            let text = std::str::from_utf8(&s)
                .map_err(|e| std::io::Error::other(format!("cursor: invalid datetime: {e}")))?;
            Ok((
                Value::DateTime(text.parse::<jiff::civil::DateTime>().map_err(|e| {
                    std::io::Error::other(format!("cursor: invalid datetime: {e}"))
                })?),
                buf,
            ))
        }
        TAG_TIME => {
            let s = read_len_prefixed(&mut buf)?;
            let text = std::str::from_utf8(&s)
                .map_err(|e| std::io::Error::other(format!("cursor: invalid time: {e}")))?;
            Ok((
                Value::Time(
                    text.parse::<jiff::civil::Time>()
                        .map_err(|e| std::io::Error::other(format!("cursor: invalid time: {e}")))?,
                ),
                buf,
            ))
        }
        TAG_BYTES => {
            let s = read_len_prefixed(&mut buf)?;
            Ok((Value::Bytes(s), buf))
        }
        TAG_RECORD => {
            let count = u32::from_le_bytes(take(&mut buf, 4)?.try_into().unwrap()) as usize;
            let mut fields = Vec::with_capacity(count.min(64));
            for _ in 0..count {
                let (field, rest) = read_value(buf)?;
                buf = rest;
                fields.push(field);
            }
            Ok((
                Value::Record(toasty_core::stmt::ValueRecord::from_vec(fields)),
                buf,
            ))
        }
        _ => Err(std::io::Error::other(format!("cursor: unknown tag byte {tag:#04x}")).into()),
    }
}

fn write_len_prefixed(bytes: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn read_len_prefixed(buf: &mut &[u8]) -> Result<Vec<u8>> {
    let len = u32::from_le_bytes(take(buf, 4)?.try_into().unwrap()) as usize;
    Ok(take(buf, len)?.to_vec())
}

fn take<'a>(buf: &mut &'a [u8], n: usize) -> Result<&'a [u8]> {
    if buf.len() < n {
        return Err(std::io::Error::other("cursor: unexpected end of payload").into());
    }
    let (head, rest) = buf.split_at(n);
    *buf = rest;
    Ok(head)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn hex_decode(token: &str) -> Result<Vec<u8>> {
    let chars: Vec<char> = token.chars().collect();
    if !chars.len().is_multiple_of(2) {
        return Err(std::io::Error::other("cursor: odd-length hex token").into());
    }
    let mut out = Vec::with_capacity(chars.len() / 2);
    for pair in chars.chunks(2) {
        let hi = pair[0]
            .to_digit(16)
            .ok_or_else(|| std::io::Error::other("cursor: invalid hex digit"))?;
        let lo = pair[1]
            .to_digit(16)
            .ok_or_else(|| std::io::Error::other("cursor: invalid hex digit"))?;
        out.push(((hi << 4) | lo) as u8);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use toasty_core::stmt::ValueRecord;

    fn round_trip(value: Value) {
        let token = encode(&value).expect("encode");
        assert!(
            token.bytes().all(|b| b.is_ascii_hexdigit()),
            "token must be hex-safe, got {token}"
        );
        assert_eq!(
            decode(&token).expect("decode"),
            value,
            "round-trip {value:?}"
        );
    }

    #[test]
    fn round_trips_scalar_variants_exactly() {
        round_trip(Value::Null);
        round_trip(Value::Bool(true));
        round_trip(Value::Bool(false));
        round_trip(Value::I8(-8));
        round_trip(Value::I16(-16));
        round_trip(Value::I32(-32));
        round_trip(Value::I64(i64::MIN));
        round_trip(Value::U8(255));
        round_trip(Value::U16(u16::MAX));
        round_trip(Value::U32(u32::MAX));
        round_trip(Value::U64(u64::MAX));
        round_trip(Value::F32(1.5));
        round_trip(Value::F64(f64::MIN));
        round_trip(Value::String("Ada Lovelace".to_string()));
        round_trip(Value::String("with spaces & symbols ?#=/".to_string()));
        round_trip(Value::Uuid(uuid::Uuid::nil()));
        round_trip(Value::Uuid(uuid::Uuid::new_v4()));
        round_trip(Value::Timestamp(
            "2024-01-15T09:30:00Z".parse().expect("timestamp"),
        ));
        round_trip(Value::Date("2024-01-15".parse().expect("date")));
        round_trip(Value::DateTime(
            "2024-01-15T09:30:00".parse().expect("datetime"),
        ));
        round_trip(Value::Time("09:30:00".parse().expect("time")));
    }

    #[test]
    fn round_trips_record_cursor_shape() {
        // The engine's multi-column cursor: [sort value, primary key]
        let cursor = Value::Record(ValueRecord::from_vec(vec![
            Value::String("Alan Turing".to_string()),
            Value::Uuid(uuid::Uuid::new_v4()),
        ]));
        round_trip(cursor);
    }

    #[test]
    fn rejects_malformed_tokens() {
        assert!(decode("").is_err(), "empty token must fail");
        assert!(decode("zz").is_err(), "non-hex must fail");
        assert!(decode("abc").is_err(), "odd length must fail");
        // Valid hex, wrong version byte
        let token = hex_encode(&[9, TAG_I64]);
        assert!(decode(&token).is_err(), "wrong version must fail");
        // Truncated payload
        let full = encode(&Value::I64(42)).expect("encode");
        let truncated = hex_decode(&full).expect("hex");
        let token = hex_encode(&truncated[..truncated.len() - 2]);
        assert!(decode(&token).is_err(), "truncation must fail");
    }

    #[test]
    fn round_trips_bytes_blobs() {
        // SQL drivers hand the primary-key back as a byte blob (e.g. a UUID
        // stored in a BLOB column) — the cursor must survive unchanged.
        round_trip(Value::Bytes(vec![1, 160, 84, 22, 172, 65, 127]));
    }

    #[test]
    fn rejects_unsupported_variants() {
        assert!(encode(&Value::List(vec![Value::I64(1)])).is_err());
    }
}
