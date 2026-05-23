export interface TranscriptMessage {
  role: string;
  content?: string | Array<Record<string, unknown>>;
  name?: string;
  tool_calls?: Array<Record<string, unknown>>;
  tool_call_id?: string;
}

export interface CompileOptions {
  session_id?: string;
  token_budget?: number;
  keep_recent_tool_results?: number;
  error_compaction_after_turns?: number;
  enable_consumed_masking?: boolean;
  run_sufficiency_check?: boolean;
  infer_subgoals?: boolean;
  soft_sufficiency?: boolean;
  subgoals?: Array<{
    id: string;
    description: string;
    required_slots?: string[];
  }>;
}

export interface CompileStats {
  input_tokens: number;
  output_tokens: number;
  tokens_saved: number;
  reduction_percent: number;
  blocks_in: number;
  blocks_out: number;
  transforms_applied: string[];
}

export interface CompileResult {
  blocks: Array<Record<string, unknown>>;
  messages: TranscriptMessage[];
  stats: CompileStats;
  sufficient: boolean;
  sufficiency_message?: string;
}

export interface AnalyzeReport {
  total_tokens: number;
  block_count: number;
  tokens_by_kind: Record<string, number>;
}

export interface FoldRecord {
  subgoal: string;
  status: "success" | "partial" | "failed";
  artifacts?: Array<{ type: string; path: string; hash?: string }>;
  preconditions_preserved?: string[];
  decisions?: string[];
  open_issues?: string[];
  token_budget_used?: number;
}
