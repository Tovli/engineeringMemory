//! Pure filter predicates applied in the application layer (ADR-0002).
//! Project (eq) / tags (multi-valued AND) / source (eq), and drop deleted docs (E9).

use crate::ingestion::domain::DocumentStatus;
use crate::retrieval::domain::query::MetadataFilter;
use crate::retrieval::ports::{DocMeta, RawSearchResult};

/// Does this candidate hit survive the filters? `doc` is the owning document's metadata.
pub fn passes(filter: &MetadataFilter, raw: &RawSearchResult, doc: &DocMeta) -> bool {
    // E9: never surface chunks of a soft-deleted document.
    if doc.status == DocumentStatus::Deleted {
        return false;
    }
    if let Some(project) = &filter.project {
        if doc.project.as_deref() != Some(project.as_str()) {
            return false;
        }
    }
    // Multi-valued AND: the document must carry every requested tag (ADR-0002).
    if !filter.tags.iter().all(|t| doc.tags.contains(t)) {
        return false;
    }
    if let Some(source) = &filter.source {
        if &raw.source_path != source {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn raw(source: &str) -> RawSearchResult {
        RawSearchResult {
            chunk_id: "chunk_1".into(),
            document_id: "doc_1".into(),
            source_path: source.into(),
            distance: 0.1,
            preview: "p".into(),
            heading_path: vec![],
            metadata: BTreeMap::new(),
        }
    }
    fn doc(project: Option<&str>, tags: &[&str], source: &str, status: DocumentStatus) -> DocMeta {
        DocMeta {
            project: project.map(String::from),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            source_path: source.into(),
            status,
        }
    }

    #[test]
    fn empty_filter_passes_active_doc() {
        let f = MetadataFilter::default();
        assert!(passes(&f, &raw("a.md"), &doc(None, &[], "a.md", DocumentStatus::Active)));
    }

    #[test]
    fn deleted_doc_never_passes() {
        let f = MetadataFilter::default();
        assert!(!passes(&f, &raw("a.md"), &doc(None, &[], "a.md", DocumentStatus::Deleted)));
    }

    #[test]
    fn project_filter_matches_exactly() {
        let f = MetadataFilter { project: Some("flexid".into()), ..Default::default() };
        let d_yes = doc(Some("flexid"), &[], "a.md", DocumentStatus::Active);
        let d_no = doc(Some("other"), &[], "a.md", DocumentStatus::Active);
        let d_none = doc(None, &[], "a.md", DocumentStatus::Active);
        assert!(passes(&f, &raw("a.md"), &d_yes));
        assert!(!passes(&f, &raw("a.md"), &d_no));
        assert!(!passes(&f, &raw("a.md"), &d_none));
    }

    #[test]
    fn tag_filter_requires_all_tags() {
        let f = MetadataFilter { tags: vec!["arch".into(), "adr".into()], ..Default::default() };
        let has_both = doc(None, &["arch", "adr", "x"], "a.md", DocumentStatus::Active);
        let has_one = doc(None, &["arch"], "a.md", DocumentStatus::Active);
        assert!(passes(&f, &raw("a.md"), &has_both));
        assert!(!passes(&f, &raw("a.md"), &has_one));
    }

    #[test]
    fn source_filter_matches_hit_path() {
        let f = MetadataFilter { source: Some("docs/auth.md".into()), ..Default::default() };
        let d = doc(None, &[], "docs/auth.md", DocumentStatus::Active);
        assert!(passes(&f, &raw("docs/auth.md"), &d));
        assert!(!passes(&f, &raw("docs/other.md"), &d));
    }

    #[test]
    fn combined_filters_and_together() {
        let f = MetadataFilter {
            project: Some("flexid".into()),
            tags: vec!["arch".into()],
            source: Some("a.md".into()),
        };
        let d = doc(Some("flexid"), &["arch"], "a.md", DocumentStatus::Active);
        assert!(passes(&f, &raw("a.md"), &d));
        // one mismatch (tag) fails the whole conjunction
        let d2 = doc(Some("flexid"), &["other"], "a.md", DocumentStatus::Active);
        assert!(!passes(&f, &raw("a.md"), &d2));
    }
}
