# Local Development Setup

## Configure Actionstep

The backend fetches asset data from Actionstep each time `GET /api/assets` is
requested. Sign in through Actionstep's authorization-code flow; OAuth tokens
are kept in backend memory only. Asset changes are written directly to
Actionstep; there are no local JSON fallbacks or response caches.

The portal supports adding an asset, editing `Asset_Value`, `Asset_Description`,
`ReportAcctNo`, `Type`, `Comments_for_Client`, `LienAmount`, and `Equity_Basis`,
and marking an asset deleted. New assets are created by POSTing a data
collection record and then PUTting its field values. Only records created by
the portal get `Client_Added=T`; later edits never change that marker. Delete
sets `Deleted=DELETED` for auditing, and deleted rows are omitted from the
display. Current edits target the action and data collection configured in
`ASSET_API_URL` (the staging sample is action 7546 / collection 105).

Create a gitignored `.env` file in the project root:

```env
APP_ENV=development
ACTIONSTEP_TOKEN_ENDPOINT=https://api.actionstepstaging.com/api/oauth/token
ACTIONSTEP_AUTHORITY=https://go.actionstepstaging.com
ACTIONSTEP_CLIENT_ID=your_client_id
ACTIONSTEP_CLIENT_SECRET=your_client_secret
ACTIONSTEP_SCOPE=all
ACTIONSTEP_REDIRECT_HOST=127.0.0.1
ACTIONSTEP_REDIRECT_PORT=7890
ACTIONSTEP_FRONTEND_URL=http://localhost:5173/
```

Keep the client secret out of source control and browser code. Do not commit the
`.env` file. Production deployments should configure these values using the
deployment platform's secret manager instead. Register
`http://127.0.0.1:7890/` as the OAuth redirect URI for the Actionstep app. The
backend listens on that loopback address for the OAuth callback while it runs.
The OAuth tokens and browser handoff are held in memory and will be lost when
the backend restarts.

## Run the app

Start the backend from `backend/`:

```bash
cargo run
```

In a second terminal, start the frontend from `frontend/`:

```bash
npm install
npm run dev
```

Open the URL printed by Vite (typically `http://localhost:5173`). The Svelte
development server proxies `/api` requests to the Rust backend on port 3000.

This app defaults to production behavior (including `Secure` session cookies)
unless explicitly told otherwise. For local plain HTTP development, set
`APP_ENV=development`. If `APP_ENV` is unset, production behavior is used.
Valid values are `production` and `development`; any other value causes startup
to fail with an error.
