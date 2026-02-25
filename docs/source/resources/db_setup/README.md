# Database Setup for Conversation State Storage

This directory contains SQL scripts needed to set up database tables for storing conversation state when using the OpenAI Responses API.

## Prerequisites

- PostgreSQL database (Supabase or self-hosted)
- Database connection credentials
- `psql` CLI tool or database admin access

## Setup Instructions

### Option 1: Using psql

```bash
psql $DATABASE_URL -f docs/db_setup/conversation_states.sql
```

### Option 2: Using Supabase Dashboard

1. Log in to your Supabase project dashboard
2. Navigate to the SQL Editor
3. Copy and paste the contents of `conversation_states.sql`
4. Run the query

### Option 3: Direct Database Connection

Connect to your PostgreSQL database using your preferred client and execute the SQL from `conversation_states.sql`.

## Verification

After running the setup, verify the table was created:

```sql
SELECT tablename FROM pg_tables WHERE tablename = 'conversation_states';
```

You should see `conversation_states` in the results.

## Configuration

After setting up the database table, configure your application to use Supabase storage by setting the appropriate environment variable or configuration parameter with your database connection string.

### Supabase Connection String

**Important:** Supabase requires different connection strings depending on your network:

- **IPv4 Networks (Most Common)**: Use the **Session Pooler** connection string (port 5432):
  ```
  postgresql://postgres.[PROJECT-REF]:[PASSWORD]@aws-0-[REGION].pooler.supabase.com:5432/postgres
  ```

- **IPv6 Networks**: Use the direct connection (port 5432):
  ```
  postgresql://postgres:[PASSWORD]@db.[PROJECT-REF].supabase.co:5432/postgres
  ```

**How to get your connection string:**
1. Go to your Supabase project dashboard
2. Settings → Database → Connection Pooling
3. Copy the **Session mode** connection string
4. Replace `[YOUR-PASSWORD]` with your actual database password
5. URL-encode special characters in the password (e.g., `#` becomes `%23`)

**Example:**
```bash
# If your password is "P@ss#123", encode it as "P%40ss%23123"
export DATABASE_URL="postgresql://postgres.[YOUR-PROJECT-REF]:<your-url-encoded-password>@aws-0-[REGION].pooler.supabase.com:5432/postgres"
```

### Testing the Connection

To test your connection string works:
```bash
export TEST_DATABASE_URL="your-connection-string-here"
cd crates/brightstaff
cargo test supabase -- --nocapture
```

## Table Schema

The `conversation_states` table stores:
- `response_id` (TEXT, PRIMARY KEY): Unique identifier for each conversation
- `input_items` (JSONB): Array of conversation messages and context
- `created_at` (BIGINT): Unix timestamp when conversation started
- `model` (TEXT): Model name used for the conversation
- `provider` (TEXT): LLM provider name
- `updated_at` (TIMESTAMP): Last update time (auto-managed)

## Maintenance

### Cleanup Old Conversations

To prevent unbounded growth, consider periodically cleaning up old conversation states:

```sql
-- Delete conversations older than 7 days
DELETE FROM conversation_states
WHERE updated_at < NOW() - INTERVAL '7 days';
```

You can automate this with a cron job or database trigger.

## Troubleshooting

If you encounter errors on first use:
- **"Table 'conversation_states' does not exist"**: Run the setup SQL
- **Connection errors**: Verify your DATABASE_URL is correct
- **Permission errors**: Ensure your database user has CREATE TABLE privileges
