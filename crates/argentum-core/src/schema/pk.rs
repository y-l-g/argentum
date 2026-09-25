//! Primary-key bridge — URL ids to typed Toasty predicates.
//!
//! Sole home of the `pk_*` helpers: parsing path-segment ids into the
//! model's key type and building equality/`IN` predicates through
//! `toasty_core` (upstream #114). Generic callers (panel loaders) cannot
//! build the typed expressions without this dynamic-value bridge.

/// Parse a URL path segment into `M`'s primary-key value.
///
/// The PK's application type decides the [`stmt::Value`] variant (`Uuid` PK
/// → `Value::Uuid`, `i64` PK → `Value::I64`, …). Returns `None` when `id`
/// does not parse (an unparseable id cannot exist — callers render
/// not-found) or the PK is not a single primitive field (composite keys
/// have no URL representation in Argentum; row keys are plain `String`s).
///
/// Bridge helper — reaches into `toasty_core` for the schema walk and the
/// untyped equality construction (upstream issue #114): the facade's
/// `find_by_primary_key` takes a typed `Expr<M::PrimaryKey>`, which generic
/// code cannot build from a `String` without a dynamic-value bridge.
fn pk_field_value<M>(
    id: &str,
) -> Option<(toasty_core::schema::app::FieldId, toasty_core::stmt::Value)>
where
    M: toasty::schema::Model,
{
    let app_model = M::schema();
    let root = app_model.as_root()?;
    if root.primary_key.fields.len() != 1 {
        return None;
    }
    let fid = root.primary_key.fields.first().copied()?;
    let model_field = app_model.fields().get(fid.index)?;
    let toasty_core::schema::app::FieldTy::Primitive(prim) = &model_field.ty else {
        return None;
    };
    let value = match prim.ty {
        toasty_core::stmt::Type::Uuid => toasty_core::stmt::Value::Uuid(id.parse().ok()?),
        toasty_core::stmt::Type::String => toasty_core::stmt::Value::String(id.to_string()),
        toasty_core::stmt::Type::Bool => toasty_core::stmt::Value::Bool(id.parse().ok()?),
        toasty_core::stmt::Type::I8 => toasty_core::stmt::Value::I8(id.parse().ok()?),
        toasty_core::stmt::Type::I16 => toasty_core::stmt::Value::I16(id.parse().ok()?),
        toasty_core::stmt::Type::I32 => toasty_core::stmt::Value::I32(id.parse().ok()?),
        toasty_core::stmt::Type::I64 => toasty_core::stmt::Value::I64(id.parse().ok()?),
        toasty_core::stmt::Type::U8 => toasty_core::stmt::Value::U8(id.parse().ok()?),
        toasty_core::stmt::Type::U16 => toasty_core::stmt::Value::U16(id.parse().ok()?),
        toasty_core::stmt::Type::U32 => toasty_core::stmt::Value::U32(id.parse().ok()?),
        toasty_core::stmt::Type::U64 => toasty_core::stmt::Value::U64(id.parse().ok()?),
        toasty_core::stmt::Type::F32 => toasty_core::stmt::Value::F32(id.parse().ok()?),
        toasty_core::stmt::Type::F64 => toasty_core::stmt::Value::F64(id.parse().ok()?),
        // Bytes PKs have no canonical URL text form; accept the UTF-8 bytes so
        // list/edit round-trip instead of 404ing.
        toasty_core::stmt::Type::Bytes => toasty_core::stmt::Value::Bytes(id.as_bytes().to_vec()),
        // Temporal PKs parse from their canonical string forms, in lockstep
        // with the cursor codec. Decimal/net PKs need Toasty
        // features this build doesn't enable (`rust_decimal`, `bigdecimal`,
        // `net`) and composite keys have no URL representation — both stay
        // documented limits.
        toasty_core::stmt::Type::Timestamp => toasty_core::stmt::Value::Timestamp(id.parse().ok()?),
        toasty_core::stmt::Type::Date => toasty_core::stmt::Value::Date(id.parse().ok()?),
        toasty_core::stmt::Type::Time => toasty_core::stmt::Value::Time(id.parse().ok()?),
        toasty_core::stmt::Type::DateTime => toasty_core::stmt::Value::DateTime(id.parse().ok()?),
        toasty_core::stmt::Type::Zoned => toasty_core::stmt::Value::Zoned(id.parse().ok()?),
        _ => return None,
    };
    Some((fid, value))
}

/// Whether `M`'s primary key is composite: more than one field.
/// Composite keys have no URL representation in Argentum (row keys are plain
/// `String`s), so handlers fail loudly (500, programmer error) instead of
/// 404ing every id and hiding the misconfiguration.
pub(crate) fn pk_is_composite<M>() -> bool
where
    M: toasty::schema::Model,
{
    M::schema()
        .as_root()
        .is_some_and(|root| root.primary_key.fields.len() > 1)
}

/// Equality predicate on `M`'s primary key for a URL path-segment `id` —
/// `pk == value`. `None` when the id does not parse as the PK's type or the
/// PK is not a single primitive field.
///
/// Consumers: the panel's edit/delete loaders, which filter the
/// tenant-scoped [`scoped_query`](crate::resource::scoped_query) instead of
/// fetching
/// every row and matching row keys in memory (item 1).
pub(crate) fn pk_eq_expr<M>(id: &str) -> Option<toasty::stmt::Expr<bool>>
where
    M: toasty::schema::Model,
{
    let (fid, value) = pk_field_value::<M>(id)?;
    let cond = toasty_core::stmt::Expr::eq(
        toasty_core::stmt::Expr::ref_self_field(fid),
        toasty_core::stmt::Expr::from(value),
    );
    Some(toasty::stmt::Expr::from_untyped(cond))
}

/// `IN` predicate over primary keys for a bulk id list — `pk IN (a, b, …)`.
/// `None` when any id fails to parse as the PK's type (an unparseable id
/// cannot exist, so the batch fails closed) or the PK is not a single
/// primitive field.
///
/// A single `IN` predicate, not an N-way `OR` chain: the batch is
/// still bounded by `MAX_BULK_IDS` in the bulk-delete handler.
pub(crate) fn pk_in_expr<M>(ids: &[&str]) -> Option<toasty::stmt::Expr<bool>>
where
    M: toasty::schema::Model,
{
    let mut parsed = ids.iter().map(|id| pk_field_value::<M>(id));
    let (fid, first) = parsed.next()??;
    let mut values = vec![first];
    for item in parsed {
        let (f, v) = item?;
        debug_assert_eq!(
            f.index, fid.index,
            "pk_in_expr: one model, one PK field — mixed fields are a bug"
        );
        values.push(v);
    }
    let cond = toasty_core::stmt::Expr::in_list(
        toasty_core::stmt::Expr::ref_self_field(fid),
        toasty_core::stmt::Expr::list(values),
    );
    Some(toasty::stmt::Expr::from_untyped(cond))
}

#[cfg(test)]
mod tests {

    use super::*;
    #[derive(Debug, toasty::Model)]
    struct DummyUser {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
        #[unique]
        email: String,
    }

    #[test]
    fn composite_pk_has_no_url_representation() {
        // GH #95: composite keys fail loudly (programmer error), never a
        // per-id 404 that hides the misconfiguration.
        #[derive(Debug, Clone, toasty::Model)]
        struct Pair {
            #[key]
            a: String,
            #[key]
            b: String,
            name: String,
        }
        assert!(pk_is_composite::<Pair>());
        assert!(!pk_is_composite::<DummyUser>());
        assert!(pk_eq_expr::<Pair>("anything").is_none());
    }

    #[test]
    fn pk_in_expr_builds_one_in_predicate_and_fails_closed() {
        // GH #85: single IN predicate; empty lists and unparseable ids yield
        // None (empty posts redirect with a toast before reaching here, GH
        // #151; an unparseable id cannot exist, so the batch must not
        // silently drop it — the handler maps None to 404).
        assert!(pk_in_expr::<DummyUser>(&[]).is_none());
        assert!(pk_in_expr::<DummyUser>(&["not-a-uuid"]).is_none());
        let a = uuid::Uuid::new_v4().to_string();
        let b = uuid::Uuid::new_v4().to_string();
        assert!(pk_in_expr::<DummyUser>(&[a.as_str(), b.as_str()]).is_some());
        assert!(pk_in_expr::<DummyUser>(&[a.as_str(), "not-a-uuid"]).is_none());
    }

    #[derive(Debug, Clone, toasty::Model)]
    struct TemporalPk {
        #[key]
        at: jiff::Timestamp,
        name: String,
    }

    #[derive(Debug, Clone, toasty::Model)]
    struct ZonedPk {
        #[key]
        at: jiff::Zoned,
        name: String,
    }

    #[tokio::test]
    async fn zoned_pks_parse_from_url_ids() {
        // Zoned PKs parse from canonical forms, in lockstep with the cursor
        // codec; garbage stays a 404.
        let z = jiff::civil::date(2024, 1, 15)
            .at(9, 30, 0, 0)
            .to_zoned(jiff::tz::TimeZone::UTC)
            .unwrap();
        assert!(pk_eq_expr::<ZonedPk>(&z.to_string()).is_some());
        assert!(pk_eq_expr::<ZonedPk>("not-a-time").is_none());
        // Round-trip through sqlite: the parsed value filters the row.
        let mut db = toasty::Db::builder()
            .models(toasty::models!(ZonedPk))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(ZonedPk {
            at: z.clone(),
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let mut db2 = db.clone();
        let expr = pk_eq_expr::<ZonedPk>(&z.to_string()).unwrap();
        let rows = ZonedPk::filter(expr).exec(&mut db2).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Ada");
    }

    #[tokio::test]
    async fn temporal_pks_parse_from_url_ids() {
        // Parse level: canonical forms resolve, garbage does not.
        assert!(pk_eq_expr::<TemporalPk>("2024-01-15T09:30:00Z").is_some());
        assert!(pk_eq_expr::<TemporalPk>("not-a-time").is_none());
        // Round-trip through sqlite: the parsed value filters the row.
        let mut db = toasty::Db::builder()
            .models(toasty::models!(TemporalPk))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        toasty::create!(TemporalPk {
            at: "2024-01-15T09:30:00Z".parse::<jiff::Timestamp>().unwrap(),
            name: "Ada".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let mut db2 = db.clone();
        let expr = pk_eq_expr::<TemporalPk>("2024-01-15T09:30:00Z").unwrap();
        let rows = TemporalPk::filter(expr).exec(&mut db2).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Ada");
    }
}
