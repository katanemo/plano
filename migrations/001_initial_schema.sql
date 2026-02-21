-- xproxy initial schema
-- Multi-tenant LLM proxy: users, projects, pipes, tokens, spending, usage

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- Users table
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    display_name TEXT,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_users_email ON users(email);

-- Projects table
CREATE TABLE projects (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT,
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(user_id, name)
);

CREATE INDEX idx_projects_user_id ON projects(user_id);

-- Pipes: map provider + API key to a project
CREATE TABLE pipes (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    provider TEXT NOT NULL,
    api_key_encrypted TEXT NOT NULL,
    model_filter TEXT,  -- NULL = all models for this provider, or comma-separated list
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(project_id, name)
);

CREATE INDEX idx_pipes_project_id ON pipes(project_id);
CREATE INDEX idx_pipes_provider ON pipes(provider);

-- Proxy tokens: bearer tokens that map to projects
CREATE TABLE proxy_tokens (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,  -- SHA-256 hash of the token
    name TEXT NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT true,
    expires_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_proxy_tokens_hash ON proxy_tokens(token_hash);
CREATE INDEX idx_proxy_tokens_project_id ON proxy_tokens(project_id);

-- Spending limits
CREATE TABLE spending_limits (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    entity_type TEXT NOT NULL CHECK (entity_type IN ('user', 'project')),
    entity_id UUID NOT NULL,
    period_type TEXT NOT NULL CHECK (period_type IN ('daily', 'monthly')),
    limit_cents BIGINT NOT NULL,  -- in cents
    is_active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(entity_type, entity_id, period_type)
);

CREATE INDEX idx_spending_limits_entity ON spending_limits(entity_type, entity_id);

-- Usage log: individual request records
CREATE TABLE usage_log (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id UUID NOT NULL REFERENCES users(id),
    project_id UUID NOT NULL REFERENCES projects(id),
    pipe_id UUID NOT NULL REFERENCES pipes(id),
    token_id UUID REFERENCES proxy_tokens(id),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cost_cents DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    is_streaming BOOLEAN NOT NULL DEFAULT false,
    status_code INTEGER,
    request_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_usage_log_user_id ON usage_log(user_id);
CREATE INDEX idx_usage_log_project_id ON usage_log(project_id);
CREATE INDEX idx_usage_log_created_at ON usage_log(created_at);
CREATE INDEX idx_usage_log_user_date ON usage_log(user_id, created_at);
CREATE INDEX idx_usage_log_project_date ON usage_log(project_id, created_at);

-- Spending counters: aggregated spending per period (for fast budget checks)
CREATE TABLE spending_counters (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    entity_type TEXT NOT NULL CHECK (entity_type IN ('user', 'project')),
    entity_id UUID NOT NULL,
    period_type TEXT NOT NULL CHECK (period_type IN ('daily', 'monthly')),
    period_start DATE NOT NULL,
    spent_micro_cents BIGINT NOT NULL DEFAULT 0,  -- micro-cents for precision
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(entity_type, entity_id, period_type, period_start)
);

CREATE INDEX idx_spending_counters_lookup ON spending_counters(entity_type, entity_id, period_type, period_start);

-- Model pricing cache (loaded from Portkey data, can be overridden)
CREATE TABLE model_pricing (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    input_price_per_token DOUBLE PRECISION NOT NULL,  -- price per token in cents
    output_price_per_token DOUBLE PRECISION NOT NULL,
    source TEXT NOT NULL DEFAULT 'portkey',  -- 'portkey' or 'manual'
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(provider, model)
);

CREATE INDEX idx_model_pricing_lookup ON model_pricing(provider, model);
