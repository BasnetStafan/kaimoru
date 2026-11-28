//! Routes log events to the file of the surface that served them.
//!
//! The request middleware records the surface on its span; the tag
//! layer copies it into a span extension, and each file layer's
//! filter keeps only the events of its own surface.

use app::util::logging::Surface;
use tracing::field::{Field, Visit};
use tracing::span::Attributes;
use tracing::{Event, Metadata, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, Filter};
use tracing_subscriber::registry::LookupSpan;

/// The surface of the request being served, held on its span.
#[derive(Clone, Copy)]
struct SurfaceTag(Surface);

/// Tags each request span with the surface it records.
///
/// Without the tag, the file filters could only see the surface on
/// the request-completed line itself; the tag lets every line a
/// handler emits follow it to the same file.
#[derive(Debug)]
pub struct SurfaceTagLayer;

impl<S> Layer<S> for SurfaceTagLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &tracing::span::Id, ctx: Context<'_, S>) {
        let mut visitor = SurfaceVisitor::default();
        attrs.record(&mut visitor);
        if let Some(surface) = visitor.found
            && let Some(span) = ctx.span(id)
        {
            span.extensions_mut().insert(SurfaceTag(surface));
        }
    }
}

/// Keeps only the events of one surface for its file layer.
///
/// An event belongs to a surface when it carries the surface field
/// itself (the request-completed line) or runs inside a tagged
/// request span. Events outside any request — startup, background
/// jobs — belong to the storefront file, the app's general record.
#[derive(Debug)]
pub struct SurfaceFilter {
    /// The surface this filter admits.
    surface: Surface,
}

impl SurfaceFilter {
    /// Builds the filter admitting `surface`'s events.
    pub fn new(surface: Surface) -> Self {
        Self { surface }
    }
}

impl<S> Filter<S> for SurfaceFilter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn enabled(&self, _meta: &Metadata<'_>, _ctx: &Context<'_, S>) -> bool {
        // Always true: the routing decision needs the event itself,
        // and a false here would disable the callsite for every layer.
        true
    }

    fn event_enabled(&self, event: &Event<'_>, ctx: &Context<'_, S>) -> bool {
        let mut visitor = SurfaceVisitor::default();
        event.record(&mut visitor);
        if let Some(surface) = visitor.found {
            return surface == self.surface;
        }
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope {
                if let Some(tag) = span.extensions().get::<SurfaceTag>() {
                    return tag.0 == self.surface;
                }
            }
        }
        self.surface == Surface::Storefront
    }
}

/// Reads a `surface` field off a span or an event.
#[derive(Default)]
struct SurfaceVisitor {
    /// The surface the field named, when one was present.
    found: Option<Surface>,
}

impl Visit for SurfaceVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "surface" {
            self.found = parse_surface(value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "surface" {
            // A `%`-recorded value formats as the bare string; a
            // string recorded as debug keeps its quotes. Accept both.
            let text = format!("{value:?}");
            self.found = parse_surface(text.trim_matches('"'));
        }
    }
}

/// Maps a logged surface value back to its variant.
fn parse_surface(value: &str) -> Option<Surface> {
    match value {
        "storefront" => Some(Surface::Storefront),
        "admin" => Some(Surface::Admin),
        "webhook" => Some(Surface::Webhook),
        "api" => Some(Surface::Api),
        _ => None,
    }
}
