use hive_router_config::persisted_documents::PersistedDocumentUrlTemplate;
use matchit::Router;

use crate::pipeline::persisted_documents::extract::HttpRequestContext;

use super::super::super::types::PersistedDocumentId;
use super::super::core::{
    DocumentIdSourceExtractor, ExtractionContext, PersistedDocumentExtractError,
};

/// Extracts a value from the URL path using a template.
pub(crate) struct UrlPathParamExtractor {
    pub(crate) router: Router<()>,
}

impl DocumentIdSourceExtractor for UrlPathParamExtractor {
    fn extract(&self, ctx: &ExtractionContext<'_>) -> Option<PersistedDocumentId> {
        let relative_path = ctx.relative_path()?;
        let matched = self.router.at(relative_path).ok()?;
        PersistedDocumentId::from_option(matched.params.get("id"))
    }
}

impl UrlPathParamExtractor {
    pub(crate) fn try_from_template(
        template: &PersistedDocumentUrlTemplate,
    ) -> Result<Self, PersistedDocumentExtractError> {
        // Templates are validated to start with '/', so the first split segment is always empty.
        let raw_segments: Vec<&str> = template.as_str().split('/').skip(1).collect();
        if !raw_segments.contains(&":id") {
            return Err(PersistedDocumentExtractError::MissingIdParam {
                template: template.as_str().to_string(),
            });
        }

        let mut wildcard_index = 0;
        // Converts our template syntax to `matchit` crate's syntax.
        // We do it to not rely on `matchit` and be able to change the implementation later.
        let route_segments = raw_segments.into_iter().map(|segment| match segment {
            ":id" => "{id}".to_string(),
            "*" => {
                let route_param = format!("{{_w{wildcard_index}}}");
                wildcard_index += 1;
                route_param
            }
            literal => literal.to_string(),
        });
        let matchit_template = format!("/{}", route_segments.collect::<Vec<_>>().join("/"));

        let mut router = Router::new();
        router
            .insert(matchit_template, ())
            .map_err(|error| PersistedDocumentExtractError::MatcherCompile(error.to_string()))?;

        Ok(Self { router })
    }
}

impl<'a> HttpRequestContext<'a> {
    pub fn path(&self) -> &str {
        self.path
    }
}
