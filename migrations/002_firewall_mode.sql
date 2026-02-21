-- Firewall mode: transparent proxy with API key hash identity
-- Phase 1: registered API keys for identity matching
-- Phase 2: async billing (is_priced column, custom pricing)

-- Registered API keys (hash only, for identity matching in firewall mode)
CREATE TABLE registered_api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    key_hash TEXT NOT NULL UNIQUE,
    provider TEXT NOT NULL,           -- "openai", "anthropic", etc.
    upstream_url TEXT NOT NULL,       -- "https://api.openai.com"
    display_name TEXT,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_api_keys_hash ON registered_api_keys(key_hash);
CREATE INDEX idx_api_keys_project ON registered_api_keys(project_id);

-- Custom model pricing (per-project overrides)
CREATE TABLE custom_model_pricing (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID REFERENCES projects(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    input_price_per_million DOUBLE PRECISION NOT NULL,
    output_price_per_million DOUBLE PRECISION NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(project_id, provider, model)
);

-- Extend spending_limits with enforcement mode (soft = async check, hard = sync check)
ALTER TABLE spending_limits ADD COLUMN IF NOT EXISTS enforcement TEXT NOT NULL DEFAULT 'soft';

-- Add is_priced flag to usage_log for async billing pipeline
ALTER TABLE usage_log ADD COLUMN IF NOT EXISTS is_priced BOOLEAN NOT NULL DEFAULT false;
CREATE INDEX idx_usage_unpriced ON usage_log(is_priced) WHERE is_priced = false;

-- Make pipe_id and user_id optional in usage_log for firewall mode
-- (firewall mode has no pipes concept, and user_id comes from project)
ALTER TABLE usage_log ALTER COLUMN pipe_id DROP NOT NULL;
ALTER TABLE usage_log ALTER COLUMN user_id DROP NOT NULL;
