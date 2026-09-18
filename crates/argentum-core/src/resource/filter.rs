//! Table filters: the four typed filter kinds plus the [`Filter`] seam.
//!
//! Moved verbatim from `resource.rs` (GH #133): no behavior change.

use toasty::stmt::Expr;

use crate::schema::{FieldLens, lens_field_name_and_label};

/// Select filter — exact match on a `String` field (e.g. `status = "published"`).
pub struct SelectFilter<M> {
    name: String,
    label: String,
    lens: FieldLens<M, String>,
    options: Vec<String>,
}

impl<M> std::fmt::Debug for SelectFilter<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectFilter")
            .field("name", &self.name)
            .field("label", &self.label)
            .field("options", &self.options)
            .finish()
    }
}

impl<M> Clone for SelectFilter<M> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            label: self.label.clone(),
            lens: self.lens.clone(),
            options: self.options.clone(),
        }
    }
}

impl<M> SelectFilter<M>
where
    M: toasty::schema::Model,
{
    /// Call sites read `SelectFilter::for(Post::fields().status(), vec![...])`.
    pub fn r#for(lens: FieldLens<M, String>, options: Vec<String>) -> Self {
        let (name, label) = lens_field_name_and_label(lens.clone());
        Self {
            name,
            label,
            lens,
            options,
        }
    }

    pub fn to_expr(&self, value: &str) -> Option<Expr<bool>> {
        let v = value.trim();
        if v.is_empty() {
            return None;
        }
        // Only allow values in options; otherwise ignore (no filter).
        if !self.options.is_empty() && !self.options.contains(&v.to_string()) {
            return None;
        }
        Some(self.lens.clone().eq(v.to_string()))
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn label_str(&self) -> &str {
        &self.label
    }
    pub fn options(&self) -> &[String] {
        &self.options
    }
}

/// Ternary filter — `true` / `false` / `all` (no filter) on a `bool` field.
pub struct TernaryFilter<M> {
    name: String,
    label: String,
    lens: FieldLens<M, bool>,
}

impl<M> std::fmt::Debug for TernaryFilter<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TernaryFilter")
            .field("name", &self.name)
            .field("label", &self.label)
            .finish()
    }
}

impl<M> Clone for TernaryFilter<M> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            label: self.label.clone(),
            lens: self.lens.clone(),
        }
    }
}

impl<M> TernaryFilter<M>
where
    M: toasty::schema::Model,
{
    pub fn r#for(lens: FieldLens<M, bool>) -> Self {
        let (name, label) = lens_field_name_and_label(lens.clone());
        Self { name, label, lens }
    }

    pub fn to_expr(&self, value: &str) -> Option<Expr<bool>> {
        match value.trim() {
            "true" => Some(self.lens.clone().eq(true)),
            "false" => Some(self.lens.clone().eq(false)),
            _ => None,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn label_str(&self) -> &str {
        &self.label
    }
}

/// Date filter — same-calendar-day match on a `Timestamp` field
/// (e.g. `created_at = "2024-01-15"` selects that whole day).
/// Range (`from`/`to`) support is future.
pub struct DateFilter<M> {
    name: String,
    label: String,
    lens: FieldLens<M, jiff::Timestamp>,
}

impl<M> std::fmt::Debug for DateFilter<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DateFilter")
            .field("name", &self.name)
            .field("label", &self.label)
            .finish()
    }
}

impl<M> Clone for DateFilter<M> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            label: self.label.clone(),
            lens: self.lens.clone(),
        }
    }
}

impl<M> DateFilter<M>
where
    M: toasty::schema::Model,
{
    pub fn r#for(lens: FieldLens<M, jiff::Timestamp>) -> Self {
        let (name, label) = lens_field_name_and_label(lens.clone());
        Self { name, label, lens }
    }

    /// Build the predicate for a submitted value (GH #93).
    ///
    /// Full RFC3339 timestamps match the exact instant (documented); a
    /// date-only `YYYY-MM-DD` matches the whole UTC day
    /// (`>= midnight AND < next midnight`), so rows stamped with any
    /// time-of-day still match.
    pub fn to_expr(&self, value: &str) -> Option<Expr<bool>> {
        let v = value.trim();
        if v.is_empty() {
            return None;
        }
        // Accept RFC3339 or YYYY-MM-DD (whole UTC day).
        if let Ok(ts) = v.parse::<jiff::Timestamp>() {
            return Some(self.lens.clone().eq(ts));
        }
        // Query decoding turns `+` into space, destroying numeric offsets
        // (`?filters=created_at:2024-01-15T09:30:00+02:00` arrives with a
        // space). A timestamp never legitimately contains a space, so retry
        // with `+` restored before giving up (GH #93).
        if v.contains(' ')
            && let Ok(ts) = v.replace(' ', "+").parse::<jiff::Timestamp>()
        {
            return Some(self.lens.clone().eq(ts));
        }
        if let Ok(date) = v.parse::<jiff::civil::Date>() {
            let start: jiff::Timestamp = format!("{date}T00:00:00Z").parse().ok()?;
            let end = start + jiff::Span::new().hours(24);
            return Some(self.lens.clone().ge(start).and(self.lens.clone().lt(end)));
        }
        None
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn label_str(&self) -> &str {
        &self.label
    }
}

/// Variant filter — exact match on an embedded-enum variant (e.g. `vehicule = "Moto"`).
///
/// Unlike [`SelectFilter`] (a `String` lens + options), a variant has no single
/// lens: Toasty stores it as one discriminant column plus one nullable column
/// per variant field. The caller therefore supplies prebuilt expressions —
/// typically `User::fields().vehicule().is_moto()` — one per option. Display
/// stays `TextColumn::computed` (see GH #77).
pub struct VariantFilter<M> {
    name: String,
    label: String,
    options: Vec<(String, Expr<bool>)>,
    _marker: std::marker::PhantomData<M>,
}

impl<M> std::fmt::Debug for VariantFilter<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VariantFilter")
            .field("name", &self.name)
            .field("label", &self.label)
            .field(
                "options",
                &self.options.iter().map(|(k, _)| k).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl<M> Clone for VariantFilter<M> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            label: self.label.clone(),
            options: self.options.clone(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<M> VariantFilter<M>
where
    M: toasty::schema::Model,
{
    /// Convenience alias so call sites read `VariantFilter::for("vehicule", "Véhicule", vec![...])`.
    pub fn r#for(
        name: impl Into<String>,
        label: impl Into<String>,
        options: Vec<(String, Expr<bool>)>,
    ) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            options,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn to_expr(&self, value: &str) -> Option<Expr<bool>> {
        let v = value.trim();
        if v.is_empty() {
            return None;
        }
        self.options
            .iter()
            .find(|(k, _)| k == v)
            .map(|(_, e)| e.clone())
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn label_str(&self) -> &str {
        &self.label
    }
    pub fn options(&self) -> &[(String, Expr<bool>)] {
        &self.options
    }
}

/// Filter enum — the `Table::filters` seam.
#[derive(Debug, Clone)]
pub enum Filter<M> {
    Select(SelectFilter<M>),
    Ternary(TernaryFilter<M>),
    Date(DateFilter<M>),
    Variant(VariantFilter<M>),
}

impl<M> From<SelectFilter<M>> for Filter<M> {
    fn from(v: SelectFilter<M>) -> Self {
        Filter::Select(v)
    }
}
impl<M> From<TernaryFilter<M>> for Filter<M> {
    fn from(v: TernaryFilter<M>) -> Self {
        Filter::Ternary(v)
    }
}
impl<M> From<DateFilter<M>> for Filter<M> {
    fn from(v: DateFilter<M>) -> Self {
        Filter::Date(v)
    }
}
impl<M> From<VariantFilter<M>> for Filter<M> {
    fn from(v: VariantFilter<M>) -> Self {
        Filter::Variant(v)
    }
}

impl<M> Filter<M>
where
    M: toasty::schema::Model,
{
    pub fn name(&self) -> &str {
        match self {
            Filter::Select(f) => f.name(),
            Filter::Ternary(f) => f.name(),
            Filter::Date(f) => f.name(),
            Filter::Variant(f) => f.name(),
        }
    }
    pub fn to_expr(&self, value: &str) -> Option<Expr<bool>> {
        match self {
            Filter::Select(f) => f.to_expr(value),
            Filter::Ternary(f) => f.to_expr(value),
            Filter::Date(f) => f.to_expr(value),
            Filter::Variant(f) => f.to_expr(value),
        }
    }
}

/// Convert a single filter or tuple of filters into `Vec<Filter<M>>`.
pub trait IntoFilters<M> {
    fn into_filters(self) -> Vec<Filter<M>>;
}

impl<M> IntoFilters<M> for Filter<M> {
    fn into_filters(self) -> Vec<Filter<M>> {
        vec![self]
    }
}
impl<M> IntoFilters<M> for SelectFilter<M> {
    fn into_filters(self) -> Vec<Filter<M>> {
        vec![self.into()]
    }
}
impl<M> IntoFilters<M> for TernaryFilter<M> {
    fn into_filters(self) -> Vec<Filter<M>> {
        vec![self.into()]
    }
}
impl<M> IntoFilters<M> for DateFilter<M> {
    fn into_filters(self) -> Vec<Filter<M>> {
        vec![self.into()]
    }
}
impl<M> IntoFilters<M> for VariantFilter<M> {
    fn into_filters(self) -> Vec<Filter<M>> {
        vec![self.into()]
    }
}
impl<M, A, B> IntoFilters<M> for (A, B)
where
    A: Into<Filter<M>>,
    B: Into<Filter<M>>,
{
    fn into_filters(self) -> Vec<Filter<M>> {
        vec![self.0.into(), self.1.into()]
    }
}
impl<M, A, B, C> IntoFilters<M> for (A, B, C)
where
    A: Into<Filter<M>>,
    B: Into<Filter<M>>,
    C: Into<Filter<M>>,
{
    fn into_filters(self) -> Vec<Filter<M>> {
        vec![self.0.into(), self.1.into(), self.2.into()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use toasty::Db;

    #[derive(Debug, Clone, toasty::Model)]
    struct Task {
        #[key]
        #[auto]
        id: uuid::Uuid,
        title: String,
        status: String,
        featured: bool,
        created_at: jiff::Timestamp,
    }

    #[derive(Debug, Clone, PartialEq, toasty::Embed)]
    enum Vehicule {
        Auto {
            #[shared(puissance)]
            puissance: String,
            seats: String,
        },
        Moto {
            #[shared(puissance)]
            puissance: String,
            cc: String,
        },
    }

    #[derive(Debug, Clone, toasty::Model)]
    struct Driver {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
        vehicule: Vehicule,
    }

    fn vehicule_filter() -> VariantFilter<Driver> {
        VariantFilter::r#for(
            "vehicule",
            "Véhicule",
            vec![
                ("Auto".to_string(), Driver::fields().vehicule().is_auto()),
                ("Moto".to_string(), Driver::fields().vehicule().is_moto()),
            ],
        )
    }

    #[tokio::test]
    async fn date_filter_date_only_matches_whole_day() {
        let mut db = Db::builder()
            .models(toasty::models!(Task))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        for (title, ts) in [
            ("Morning", "2024-01-15T09:30:00Z"),
            ("Night", "2024-01-15T23:59:59Z"),
            ("Next", "2024-01-16T00:00:01Z"),
        ] {
            toasty::create!(Task {
                title: title.to_string(),
                status: "draft".to_string(),
                featured: false,
                created_at: ts.parse::<jiff::Timestamp>().unwrap(),
            })
            .exec(&mut db)
            .await
            .unwrap();
        }
        let f = DateFilter::r#for(Task::fields().created_at());
        let expr = f.to_expr("2024-01-15").expect("date-only must build");
        let mut db2 = db.clone();
        let mut rows = Task::filter(expr).exec(&mut db2).await.unwrap();
        rows.sort_by(|a, b| a.title.cmp(&b.title));
        assert_eq!(
            rows.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(),
            vec!["Morning", "Night"],
            "date-only must match the whole UTC day (GH #93)"
        );
        // Exact RFC3339 instants still match exactly.
        let expr = f
            .to_expr("2024-01-15T09:30:00Z")
            .expect("rfc3339 must build");
        let rows = Task::filter(expr).exec(&mut db2).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(f.to_expr("not-a-date").is_none());
    }

    #[test]
    fn date_filter_recovers_plus_offsets_mangled_by_query_decode() {
        let f = DateFilter::r#for(Task::fields().created_at());
        // `+02:00` arrives as ` 02:00` after `+`-as-space decoding (GH #93).
        assert!(f.to_expr("2024-01-15T09:30:00 02:00").is_some());
        assert!(f.to_expr("2024-01-15T09:30:00+02:00").is_some());
        assert!(f.to_expr("not-a-date").is_none());
        assert!(f.to_expr("").is_none());
    }

    #[test]
    fn variant_filter_to_expr_contract() {
        let f = vehicule_filter();
        assert_eq!(f.name(), "vehicule");
        assert_eq!(f.label_str(), "Véhicule");
        assert!(f.to_expr("").is_none(), "empty yields no filter");
        assert!(f.to_expr("   ").is_none(), "blank yields no filter");
        assert!(
            f.to_expr("Avion").is_none(),
            "unknown yields no filter, got {:?}",
            f.to_expr("Avion").is_some()
        );
        assert!(f.to_expr("Auto").is_some(), "known variant must match");
        assert!(f.to_expr("Moto").is_some(), "known variant must match");
        // Whitespace trims like SelectFilter.
        assert!(f.to_expr("  Moto  ").is_some());
        // Via the Filter enum + IntoFilters seam.
        let via_enum: Filter<Driver> = f.clone().into();
        assert_eq!(via_enum.name(), "vehicule");
        assert!(via_enum.to_expr("Moto").is_some());
        assert!(via_enum.to_expr("nope").is_none());
        let vec = f.into_filters();
        assert_eq!(vec.len(), 1);
    }

    #[tokio::test]
    async fn variant_filter_hits_only_the_variant() {
        let mut db = Db::builder()
            .models(toasty::models!(Driver))
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db.push_schema().await.unwrap();
        // Same shared `puissance` value in both variants — the variant gate
        // must exclude the other variant (GH #77 acceptance).
        toasty::create!(Driver {
            name: "Alice",
            vehicule: Vehicule::Auto {
                puissance: "80".to_string(),
                seats: "4".to_string(),
            },
        })
        .exec(&mut db)
        .await
        .unwrap();
        toasty::create!(Driver {
            name: "Bob",
            vehicule: Vehicule::Moto {
                puissance: "80".to_string(),
                cc: "600".to_string(),
            },
        })
        .exec(&mut db)
        .await
        .unwrap();
        toasty::create!(Driver {
            name: "Cara",
            vehicule: Vehicule::Auto {
                puissance: "120".to_string(),
                seats: "2".to_string(),
            },
        })
        .exec(&mut db)
        .await
        .unwrap();

        let f = vehicule_filter();
        let mut db2 = db.clone();
        let motos = Driver::filter(f.to_expr("Moto").unwrap())
            .exec(&mut db2)
            .await
            .unwrap();
        assert_eq!(
            motos.len(),
            1,
            "Moto filter must hit one row, got {motos:?}"
        );
        assert_eq!(motos[0].name, "Bob");

        let autos = Driver::filter(f.to_expr("Auto").unwrap())
            .exec(&mut db2)
            .await
            .unwrap();
        assert_eq!(
            autos.len(),
            2,
            "Auto filter must hit two rows, got {autos:?}"
        );

        // Composes with search via AND (the loader's contract).
        let search = Driver::fields().name().starts_with("B".to_string());
        let both = search.and(f.to_expr("Moto").unwrap());
        let rows = Driver::filter(both).exec(&mut db2).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Bob");

        // Same shared value, other variant excluded.
        let both = Driver::fields()
            .name()
            .starts_with("A".to_string())
            .and(f.to_expr("Moto").unwrap());
        let rows = Driver::filter(both).exec(&mut db2).await.unwrap();
        assert!(
            rows.is_empty(),
            "Alice shares puissance 80 but is Auto, must not match Moto: {rows:?}"
        );
    }
}
