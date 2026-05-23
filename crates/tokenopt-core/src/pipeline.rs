//! Build transform pipeline from [`CompileOptions`].

use crate::compile::CompileOptions;
use crate::guideline::GuidelineBank;
use crate::transform::{
    AgentOmitTransform, BudgetTrimTransform, CachePackTransform, ConsumedResultMaskTransform,
    ErrorCompactionTransform, ExternalCompressTransform, FoldCollapseTransform,
    FoldInjectTransform, GuidelinePinTransform, MemoryPruneTransform, ReferentialKeepTransform,
    RollingWindowTransform, RuleSummarizeTransform, TransformPipeline, TransformPipelineBuilder,
};

pub fn build_pipeline(options: &CompileOptions) -> TransformPipeline {
    let mut builder = TransformPipelineBuilder::new();
    let t = &options.transforms;

    if t.referential_keep {
        builder = builder.then(ReferentialKeepTransform);
    }
    if t.guideline_bank {
        let bank = options
            .guideline_bank_path
            .as_ref()
            .and_then(|p| GuidelineBank::load_from_path(p).ok())
            .unwrap_or_else(GuidelineBank::default_builtin);
        builder = builder.then(GuidelinePinTransform::new(bank));
    }
    if !options.fold_records.is_empty() {
        builder = builder.then(FoldInjectTransform::new(options.fold_records.clone()));
    }
    if t.fold_collapse {
        builder = builder.then(FoldCollapseTransform);
    }
    if t.error_compaction {
        builder = builder.then(ErrorCompactionTransform);
    }
    if t.consumed_result_mask {
        builder = builder.then(ConsumedResultMaskTransform);
    }
    if t.agent_omit {
        builder = builder.then(AgentOmitTransform);
    }
    if t.memory_prune {
        builder = builder.then(MemoryPruneTransform);
    }
    if t.summarization {
        builder = builder.then(RuleSummarizeTransform::new(
            options.summarize_keep_recent_blocks,
        ));
    }
    if t.external_compress {
        builder = builder.then(ExternalCompressTransform::new(
            options.external_compress_url.clone(),
        ));
    }
    if t.cache_packer {
        builder = builder.then(CachePackTransform);
    }
    if t.rolling_window {
        builder = builder.then(RollingWindowTransform);
    }
    if t.budget_trim {
        builder = builder.then(BudgetTrimTransform);
    }

    builder.build()
}
