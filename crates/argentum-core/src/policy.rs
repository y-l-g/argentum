//! `Policy` — per-Resource authorization (CONTEXT.md).
//!
//! Default-deny; checked in both page and shard/procedure handlers (ADR-0004).
//! The `Policy` trait is associated with a `Resource`; `Resource::Policy`
//! defaults to `DenyAll`. Showcase resources override to `AllowAll` to keep
//! the demo usable while preserving default-deny for new resources.

use topcoat::context::Cx;

use crate::resource::Resource;

/// Authorization rules for a `Resource`.
///
/// Each method receives `&Cx` (so it can read tenancy, session, role, etc.)
/// and, where relevant, the target record. Default is deny.
pub trait Policy<R: Resource>: Send + Sync + 'static {
    fn can_view_any(_cx: &Cx) -> bool {
        false
    }
    fn can_view(_cx: &Cx, _record: &R::Model) -> bool {
        false
    }
    fn can_create(_cx: &Cx) -> bool {
        false
    }
    fn can_update(_cx: &Cx, _record: &R::Model) -> bool {
        false
    }
    fn can_delete(_cx: &Cx, _record: &R::Model) -> bool {
        false
    }
}

/// Default-deny policy — the `Resource::Policy` default.
#[derive(Debug, Clone, Copy)]
pub struct DenyAll;

impl<R: Resource> Policy<R> for DenyAll {}

/// Allow-all policy — useful for showcase/demo and tests that need open access.
#[derive(Debug, Clone, Copy)]
pub struct AllowAll;

impl<R: Resource> Policy<R> for AllowAll {
    fn can_view_any(_cx: &Cx) -> bool {
        true
    }
    fn can_view(_cx: &Cx, _record: &R::Model) -> bool {
        true
    }
    fn can_create(_cx: &Cx) -> bool {
        true
    }
    fn can_update(_cx: &Cx, _record: &R::Model) -> bool {
        true
    }
    fn can_delete(_cx: &Cx, _record: &R::Model) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use topcoat::context::CxTestBuilder;

    #[derive(Debug, toasty::Model)]
    struct Dummy {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    struct DummyResource;
    impl Resource for DummyResource {
        type Model = Dummy;
    }

    struct OpenResource;
    impl Resource for OpenResource {
        type Model = Dummy;
        fn can_view_any(_cx: &Cx) -> bool {
            true
        }
        fn can_view(_cx: &Cx, _record: &Dummy) -> bool {
            true
        }
        fn can_create(_cx: &Cx) -> bool {
            true
        }
        fn can_update(_cx: &Cx, _record: &Dummy) -> bool {
            true
        }
        fn can_delete(_cx: &Cx, _record: &Dummy) -> bool {
            true
        }
    }

    #[test]
    fn deny_all_denies() {
        let cx = CxTestBuilder::new().build();
        assert!(!DummyResource::can_view_any(&cx));
        assert!(!DummyResource::can_create(&cx));
        let rec = Dummy {
            id: uuid::Uuid::nil(),
            name: "x".into(),
        };
        assert!(!DummyResource::can_view(&cx, &rec));
        assert!(!<DenyAll as Policy<DummyResource>>::can_view_any(&cx));
    }

    #[test]
    fn allow_all_allows() {
        let cx = CxTestBuilder::new().build();
        assert!(OpenResource::can_view_any(&cx));
        assert!(OpenResource::can_create(&cx));
        assert!(<AllowAll as Policy<OpenResource>>::can_view_any(&cx));
    }
}
