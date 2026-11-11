//! `kor context [--history N]` (§5.3): the last request's context window breakdown, the
//! `[context]` thresholds and recent use. Rendering lives in `kor_runtime::context`.

use crate::Ctx;

pub fn run(ctx: &Ctx, history: usize) -> bool {
    let cfg = ctx
        .config
        .as_ref()
        .map(|c| c.config.context.clone())
        .unwrap_or_default();
    print!("{}", kor_runtime::context::report(&ctx.l, &cfg, history));
    true
}
