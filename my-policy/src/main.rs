extern crate paralegal_policy;
extern crate anyhow;

use anyhow::Result;
use std::sync::Arc;
use paralegal_policy::{Context, assert_warning, DefId, Marker, paralegal_spdg::DefKind};

macro_rules! marker {
    ($id:ident) => {
        Marker::new_intern(stringify!($id))
    };
}

trait ContextExt {
    fn marked_type<'a>(&'a self, marker: Marker) -> Box<dyn Iterator<Item = DefId> + 'a>;
}

impl ContextExt for Context {

    fn marked_type<'a>(&'a self, marker: Marker) -> Box<dyn Iterator<Item = DefId> + 'a> {
        Box::new(
            self.marked(marker)
                .filter(|(did, _)| self.desc().def_info[did].kind == DefKind::Type)
                .map(|(did, refinement)| {
                    assert!(refinement.on_self());
                    *did
                }),
        ) as Box<_>
    }
}

fn check(ctx: Arc<Context>) -> Result<()> {
    let pageview_data = ctx.marked_type(marker!(pageviews)).collect::<Vec<_>>();
    assert_warning!(ctx, !pageview_data.is_empty(), "No pageview data found. The policy may be vacuous.");
    ctx.named_policy("expiration", |ctx| {
        let found = ctx.controller_contexts().any(|ctx| {
            
        })
    });
    Ok(())
}

fn main() -> Result<()> {
    // The directory where the project-to-analyze is
    let dir = "..";
	paralegal_policy::SPDGGenCommand::global()
        .run(dir)?
        .with_context(check)
}