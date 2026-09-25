//! URL-safe encoding for Toasty pagination cursors.
//!
//! Toasty's cursor-based pagination hands back opaque `stmt::Value`s
//! (`Page::next_cursor` / `Page::prev_cursor`) that `.after()` / `.before()`
//! accept to resume the walk. Server-rendered pagination needs those cursors
//! in a URL query parameter, and Toasty does not provide a string round-trip,
//! so this module encodes the value into a self-describing byte payload and
//! hex-encodes that into an ASCII-safe token (no percent-encoding or escaping
//! ambiguity in hrefs).
//!
//! The encoding preserves the exact [`Value`] variant (an `I64` decodes as an
//! `I64`, a `Uuid` as a `Uuid`), which matters because the engine compares the
//! cursor against the ordering column's typed value. Records round-trip: a
//! multi-column cursor encodes and decodes recursively, with decode capped at
//! `MAX_CURSOR_DEPTH` nesting levels so a tampered token cannot drive unbounded
//! recursion. The variants with no tag — lists, objects, decimals — return an
//! error rather than silently degrading, since an unsortable column has no
//! business in a cursor anyway.

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
const TAG_ZONED: u8 = b'z';
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

/// A malformed cursor token — the `?after=`/`?before=` value itself is bad —
/// or a conflicting cursor pair (`?after=` + `?before=` together, GH #155).
///
/// One of the two markers [`is_cursor_error`] reads to drop the cursor from
/// the retry link (GH #110): retrying the identical URL can never succeed,
/// while a transient failure must retry the same evidence (GH #98). The
/// message is the decode error's own `cursor: …` text, or the conflict
/// message's.
#[derive(Debug)]
pub(crate) struct CursorDecodeError(String);

impl CursorDecodeError {
    /// `?after=` and `?before=` together: Toasty cursor pagination takes
    /// exactly one cursor, so the pair can never resolve — the same retry
    /// contract as a malformed token (drop pagination, keep the rest).
    pub(crate) fn conflicting_cursors() -> topcoat::Error {
        topcoat::Error::from(CursorDecodeError(
            "cursor: after and before are mutually exclusive".to_string(),
        ))
    }
}

impl std::fmt::Display for CursorDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CursorDecodeError {}

/// A cursor the query's ordering refuses (GH #294): the token decodes, but it
/// was cut from a different `ORDER BY` — the sort changed since the link was
/// built — so the engine rejects the statement. Distinct from
/// [`CursorDecodeError`] because the token itself is well formed; the two share
/// the retry contract below.
#[derive(Debug)]
pub(crate) struct CursorRejectedError(String);

impl CursorRejectedError {
    /// Attribute a failed paginated load to `error`, the engine's refusal of
    /// the request's cursor.
    pub(crate) fn rejected(error: &topcoat::Error) -> topcoat::Error {
        topcoat::Error::from(CursorRejectedError(error.to_string()))
    }
}

impl std::fmt::Display for CursorRejectedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CursorRejectedError {}

/// Whether `error` is the request's cursor's fault (GH #110, GH #294): a
/// malformed token, a conflicting cursor pair, or a token the ordering
/// rejects. Retrying the identical request can never succeed for any of them,
/// so the list page drops the cursor from its retry link and the live retry
/// resets it.
pub(crate) fn is_cursor_error(error: &topcoat::Error) -> bool {
    error.downcast_ref::<CursorDecodeError>().is_some()
        || error.downcast_ref::<CursorRejectedError>().is_some()
}

/// Decode a token produced by [`encode`] back into a cursor [`Value`].
///
/// # Errors
///
/// Errors on malformed input (wrong length, unknown tag or version) so a
/// tampered or truncated `?after=`/`?before=` parameter fails loudly instead
/// of silently restarting pagination. Record nesting is depth-capped (GH #95)
/// so attacker-controlled tokens cannot drive unbounded recursion. Every
/// failure carries the crate-private `CursorDecodeError` marker, letting the
/// list page's retry link tell a tampered cursor (drop it) from a transient
/// load failure (keep it, #98).
pub fn decode(token: &str) -> Result<Value> {
    decode_inner(token).map_err(|e| topcoat::Error::from(CursorDecodeError(e.to_string())))
}

fn decode_inner(token: &str) -> Result<Value> {
    let payload = hex_decode(token)?;
    let mut buf = &payload[..];
    let version = take::<1>(&mut buf)?[0];
    if version != VERSION {
        return Err(std::io::Error::other(format!("cursor: unsupported version {version}")).into());
    }
    let (value, rest) = read_value_with_depth(buf, 0)?;
    if !rest.is_empty() {
        return Err(std::io::Error::other("cursor: trailing bytes after value").into());
    }
    Ok(value)
}

/// Max nested-record depth accepted on decode (GH #95).
const MAX_CURSOR_DEPTH: usize = 16;

/// Generate the codec for the variants with a uniform payload from one table.
///
/// `bytes` rows are little-endian fixed-width scalars; `text` rows are
/// length-prefixed `to_string` spellings parsed back into their jiff type,
/// with `as "<label>"` naming the type in a decode error. `write_tagged` and
/// `read_tagged` expand from the same rows, so a variant cannot reach one side
/// of the codec only. The write side reports whether it encoded `value`; the
/// read side reports whether it knows `tag`, leaving the variants with their
/// own framing (`Null`, `Bool`, `String`, `Uuid`, `Bytes`, `Record`) to the
/// matches below.
macro_rules! cursor_tags {
    (
        bytes: $( $btag:ident => $bv:ident($bty:ty) ),* $(,)? ;
        text: $( $ttag:ident => $tv:ident($tty:ty) as $tlabel:literal ),* $(,)? ;
    ) => {
        fn write_tagged(value: &Value, out: &mut Vec<u8>) -> Result<bool> {
            match value {
                $(
                    Value::$bv(v) => {
                        out.push($btag);
                        out.extend_from_slice(&v.to_le_bytes());
                    }
                )*
                $(
                    Value::$tv(v) => {
                        out.push($ttag);
                        write_len_prefixed(v.to_string().as_bytes(), out)?;
                    }
                )*
                _ => return Ok(false),
            }
            Ok(true)
        }

        fn read_tagged(tag: u8, buf: &mut &[u8]) -> Result<Option<Value>> {
            Ok(Some(match tag {
                $(
                    $btag => Value::$bv(<$bty>::from_le_bytes(
                        take::<{ std::mem::size_of::<$bty>() }>(buf)?,
                    )),
                )*
                $(
                    $ttag => {
                        let s = read_len_prefixed(buf)?;
                        let text = std::str::from_utf8(&s).map_err(|e| {
                            std::io::Error::other(format!("cursor: invalid {}: {e}", $tlabel))
                        })?;
                        Value::$tv(text.parse::<$tty>().map_err(|e| {
                            std::io::Error::other(format!("cursor: invalid {}: {e}", $tlabel))
                        })?)
                    }
                )*
                _ => return Ok(None),
            }))
        }
    };
}

cursor_tags! {
    bytes: TAG_I8 => I8(i8), TAG_I16 => I16(i16), TAG_I32 => I32(i32), TAG_I64 => I64(i64),
        TAG_U8 => U8(u8), TAG_U16 => U16(u16), TAG_U32 => U32(u32), TAG_U64 => U64(u64),
        TAG_F32 => F32(f32), TAG_F64 => F64(f64);
    text: TAG_TIMESTAMP => Timestamp(jiff::Timestamp) as "timestamp",
        TAG_DATE => Date(jiff::civil::Date) as "date",
        TAG_DATETIME => DateTime(jiff::civil::DateTime) as "datetime",
        TAG_TIME => Time(jiff::civil::Time) as "time",
        TAG_ZONED => Zoned(jiff::Zoned) as "zoned";
}

fn write_value(value: &Value, out: &mut Vec<u8>) -> Result<()> {
    if write_tagged(value, out)? {
        return Ok(());
    }
    match value {
        Value::Null => out.push(TAG_NULL),
        Value::Bool(b) => {
            out.push(TAG_BOOL);
            out.push(u8::from(*b));
        }
        Value::String(v) => {
            out.push(TAG_STRING);
            write_len_prefixed(v.as_bytes(), out)?;
        }
        Value::Uuid(v) => {
            out.push(TAG_UUID);
            out.extend_from_slice(v.as_bytes());
        }
        Value::Bytes(v) => {
            out.push(TAG_BYTES);
            write_len_prefixed(v, out)?;
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

/// Reads one tagged value with depth tracking; returns it plus the remaining buffer.
fn read_value_with_depth(buf: &[u8], depth: usize) -> Result<(Value, &[u8])> {
    if depth > MAX_CURSOR_DEPTH {
        return Err(std::io::Error::other("cursor: record nesting too deep").into());
    }
    let mut buf = buf;
    let tag = take::<1>(&mut buf)?[0];
    if let Some(value) = read_tagged(tag, &mut buf)? {
        return Ok((value, buf));
    }
    match tag {
        TAG_NULL => Ok((Value::Null, buf)),
        TAG_BOOL => {
            let b = take::<1>(&mut buf)?[0];
            match b {
                0 => Ok((Value::Bool(false), buf)),
                1 => Ok((Value::Bool(true), buf)),
                _ => Err(std::io::Error::other("cursor: invalid bool byte").into()),
            }
        }
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
            let bytes = take::<16>(&mut buf)?;
            Ok((
                Value::Uuid(
                    uuid::Uuid::from_slice(&bytes)
                        .map_err(|e| std::io::Error::other(format!("cursor: invalid uuid: {e}")))?,
                ),
                buf,
            ))
        }
        TAG_BYTES => {
            let s = read_len_prefixed(&mut buf)?;
            Ok((Value::Bytes(s), buf))
        }
        TAG_RECORD => {
            let count = u32::from_le_bytes(take::<4>(&mut buf)?) as usize;
            let mut fields = Vec::with_capacity(count.min(64));
            for _ in 0..count {
                let (field, rest) = read_value_with_depth(buf, depth + 1)?;
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

/// Write a `u32` length prefix followed by `bytes`.
///
/// The length is the frame's own count, so a value longer than `u32::MAX`
/// cannot be represented: it errors instead of writing a truncated prefix that
/// would decode as a different, shorter frame (GH #212).
fn write_len_prefixed(bytes: &[u8], out: &mut Vec<u8>) -> Result<()> {
    let len = u32::try_from(bytes.len())
        .map_err(|e| std::io::Error::other(format!("cursor: value too long: {e}")))?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn read_len_prefixed(buf: &mut &[u8]) -> Result<Vec<u8>> {
    let len = u32::from_le_bytes(take::<4>(buf)?) as usize;
    Ok(take_slice(buf, len)?.to_vec())
}

/// Take exactly `N` bytes off the front, or fail closed.
///
/// `N` is a const parameter so a fixed-width decode's length is settled by the
/// type rather than by a reader checking that the preceding `take` asked for
/// the right count: `split_first_chunk` hands back the array, and the
/// `from_le_bytes` conversions at the call sites need no fallible step
/// (GH #212).
fn take<const N: usize>(buf: &mut &[u8]) -> Result<[u8; N]> {
    let Some((head, rest)) = buf.split_first_chunk::<N>() else {
        return Err(std::io::Error::other("cursor: unexpected end of payload").into());
    };
    *buf = rest;
    Ok(*head)
}

/// Take `n` bytes off the front, or fail closed — the length-prefixed payload's
/// runtime length, which has no const to pin it.
fn take_slice<'a>(buf: &mut &'a [u8], n: usize) -> Result<&'a [u8]> {
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
    use toasty_core::stmt::ValueRecord;

    use super::*;

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
        round_trip(Value::Zoned(
            jiff::civil::date(2024, 1, 15)
                .at(9, 30, 0, 0)
                .to_zoned(jiff::tz::TimeZone::UTC)
                .expect("zoned"),
        ));
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

    /// The wire format is a contract with tokens already in browsers' URLs, so
    /// its exact bytes are pinned here instead of only round-tripped: a
    /// symmetric tag swap or payload-width change round-trips cleanly and still
    /// breaks every URL in flight. `VERSION` is the escape hatch — bump it for a
    /// deliberate layout change and update this token with it; a token that
    /// stops matching this test without a version bump is a bug.
    #[test]
    fn the_wire_format_is_pinned() {
        assert_eq!(VERSION, 1);

        // The engine's multi-column cursor, [sort value, primary key], sized to
        // cross the record, i64, string and uuid tags.
        let value = Value::Record(ValueRecord::from_vec(vec![
            Value::I64(42),
            Value::String("Ada Lovelace".to_string()),
            Value::Uuid(uuid::Uuid::nil()),
        ]));
        let token = encode(&value).expect("encode");
        assert_eq!(
            token,
            concat!(
                "017203000000382a00000000000000730c000000416461204c6f76656c616365",
                "7500000000000000000000000000000000",
            )
        );
        assert_eq!(decode(&token).expect("decode"), value);
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

    #[test]
    fn rejects_overly_deep_record_nesting() {
        let mut value = Value::I64(1);
        for _ in 0..(MAX_CURSOR_DEPTH + 2) {
            value = Value::Record(ValueRecord::from_vec(vec![value]));
        }
        let token = encode(&value).expect("deep encode must succeed");
        assert!(
            decode(&token).is_err(),
            "decode must cap record nesting at {MAX_CURSOR_DEPTH}"
        );
    }
}
