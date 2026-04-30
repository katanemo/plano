# PlanoHelper

A lightweight Slack bot (Next.js app on Vercel) that handles slash commands for
Plano ops work. Today it ships one command:

- `/update-providers` — refresh `crates/hermesllm/src/bin/provider_models.yaml`
and open a PR.

## How it works

```
Slack user → /update-providers
  → Vercel (this app): verify signature, check user allowlist, ack in <3s
    → GitHub repository_dispatch (event_type: update-providers)
      → GitHub Actions: run fetch_models, open PR, reply to Slack response_url
```

Slack slash commands must be acknowledged within 3 seconds, so Vercel only
dispatches the workflow and returns an ephemeral ack. The GitHub Actions
workflow (`.github/workflows/update-providers.yml`) does the heavy lifting and
posts the PR link back to the same Slack thread via the `response_url`.

## Adding a new slash command

1. Create `src/lib/commands/<name>.ts` exporting a `SlashCommand`.
2. Add it to the array in `src/lib/commands/registry.ts`.
3. Register the command in your Slack app's "Slash Commands" section with the
  same name (`/your-command`) pointed at
   `https://<vercel-domain>/api/slack/commands`.
4. If the handler dispatches a workflow, add a matching workflow under
  `.github/workflows/` listening for that `repository_dispatch` event type.

The Slack route (`src/app/api/slack/commands/route.ts`) handles signature
verification and user allowlisting once for every command.

## Setup

### 1. Create the Slack app

From [https://api.slack.com/apps](https://api.slack.com/apps) → **Create New App** → **From scratch**,
name it **PlanoHelper**.

- **Slash Commands** → **Create New Command**
  - Command: `/update-providers`
  - Request URL: `https://<your-vercel-domain>/api/slack/commands`
  - Short description: `Refresh provider_models.yaml and open a PR`
  - Usage hint: *(leave blank)*
- **OAuth & Permissions** → Bot Token Scopes: `commands` is the only required scope.
- Install the app to your workspace.
- Copy the **Signing Secret** from **Basic Information**.

### 2. Configure Vercel

Create a **new, standalone Vercel project** (not attached to the marketing
sites) pointing at this monorepo. Set **Root Directory** to `apps/planohelper`.
Framework preset is auto-detected as Next.js.

Recommended project settings:

- **Deployment Protection** → enable **Vercel Authentication** for **Preview**
deploys. Production stays public so Slack can reach it.
- **Environment Variables** → add the values below, scoped to **Production**
only (preview deploys don't need a working `GITHUB_TOKEN`).


| Name                     | Value                                                    |
| ------------------------ | -------------------------------------------------------- |
| `SLACK_SIGNING_SECRET`   | From the Slack app's Basic Information page              |
| `SLACK_ALLOWED_USER_IDS` | Comma-separated Slack user IDs, e.g. `U01ABCDE,U02FGHIJ` |
| `GITHUB_TOKEN`           | Fine-grained PAT — see step 4                            |
| `GITHUB_REPO`            | `katanemo/plano`                                         |


After deploy, the health probe at `https://<vercel-domain>/api/health` should
return `{ ok: true }`.

### 3. Configure GitHub repo secrets

Under **Settings → Secrets and variables → Actions** add:

- Provider API keys: `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `DEEPSEEK_API_KEY`,
`GROK_API_KEY`, `DASHSCOPE_API_KEY`, `MOONSHOT_API_KEY`, `ZHIPU_API_KEY`,
`GOOGLE_API_KEY`
- AWS for Bedrock: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION`

Missing keys are OK — `fetch_models` skips the corresponding providers.

### 4. GitHub PAT for `GITHUB_TOKEN`

Create a **fine-grained** PAT at
[https://github.com/settings/personal-access-tokens/new](https://github.com/settings/personal-access-tokens/new):

- **Resource owner**: `katanemo` (must be selected — defaults to your personal
account, which is a common gotcha. The org must have fine-grained PATs
enabled in its policy.)
- **Repository access**: Only select repositories → `katanemo/plano`
- **Repository permissions**:
  - **Contents: Read and write** — required to call
  `POST /repos/{owner}/{repo}/dispatches` (the `repository_dispatch` event
  this app sends).
  - *Metadata: Read-only* is auto-included.
- No other permissions are needed. The workflow itself uses GitHub's built-in
`GITHUB_TOKEN` (with `contents: write` + `pull-requests: write` declared in
the workflow) to create the branch, commit, and open the PR — your PAT does
not need PR or Actions scopes.

If your PAT can't see `katanemo/plano`, the most common causes are: Resource
owner dropdown still set to your personal account; SAML SSO session not
refreshed; or org policy restricting which repos PATs can target.

### 5. Find your Slack user ID

In Slack → your profile → **More (⋯)** → **Copy member ID**. Paste it into
`SLACK_ALLOWED_USER_IDS`.

## Local development

```bash
cd apps/planohelper
cp .env.example .env.local
# fill in values
npm run dev
```

Expose the dev server with ngrok or similar and point the Slack slash command
request URL at `https://<tunnel>/api/slack/commands`.

## Scripts

- `npm run dev` — Next.js dev server
- `npm run build` — production build
- `npm run lint` — Biome lint
- `npm run typecheck` — `tsc --noEmit`
