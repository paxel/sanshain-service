// Sanshain — shared JS utilities
// Used by admin, account, service, and dashboard pages.

let sanshainToken = localStorage.getItem('sanshain_token');
let csrfToken = null;

// --- Network error helper ---
function friendlyError(err) {
    if (err instanceof TypeError && (err.message.includes('NetworkError') || err.message.includes('Failed to fetch') || err.message.includes('Load failed'))) {
        return 'Server is not reachable. Please check that the service is running.';
    }
    return err.message || String(err);
}

// --- CSRF ---
async function fetchCsrfToken() {
    try {
        const res = await fetch('/csrf-token');
        if (res.ok) {
            const data = await res.json();
            csrfToken = data.csrf_token;
        }
    } catch (_) {}
}

// --- Authenticated API helper ---
// Automatically attaches Bearer token and CSRF token.
// If body is an object, serialises as JSON.
// On 401, clears session and calls onSessionExpired() if defined.
async function apiCall(url, options = {}) {
    const headers = { ...options.headers };
    if (sanshainToken) headers['Authorization'] = `Bearer ${sanshainToken}`;
    if (options.body && typeof options.body === 'object') {
        headers['Content-Type'] = 'application/json';
        options.body = JSON.stringify(options.body);
    }
    const method = (options.method || 'GET').toUpperCase();
    if (['POST', 'PUT', 'DELETE', 'PATCH'].includes(method) && csrfToken) {
        headers['X-CSRF-Token'] = csrfToken;
    }
    const res = await fetch(url, { ...options, headers });
    if (res.status === 401) {
        sanshainToken = null;
        localStorage.removeItem('sanshain_token');
        if (typeof onSessionExpired === 'function') onSessionExpired();
        throw new Error('Session expired');
    }
    return res;
}

// --- HTML / attribute escaping ---
function escapeHtml(str) {
    const div = document.createElement('div');
    div.textContent = str;
    return div.innerHTML;
}

function escapeAttr(str) {
    return str.replace(/\\/g, '\\\\').replace(/'/g, "\\'");
}

// --- Confirm modal ---
// Requires a #confirm-modal, #confirm-message, #confirm-yes in the page.
function confirmAction(message, onConfirm) {
    document.getElementById('confirm-message').innerHTML = message;
    document.getElementById('confirm-modal').classList.remove('hidden');
    document.getElementById('confirm-yes').onclick = () => {
        closeConfirmModal();
        onConfirm();
    };
}
// Alias used by admin page
const confirmDelete = confirmAction;

function closeConfirmModal() {
    document.getElementById('confirm-modal').classList.add('hidden');
}

// --- Version badge ---
function loadVersionBadge(elementId) {
    fetch('/version')
        .then(r => r.json())
        .then(data => {
            const el = document.getElementById(elementId);
            if (el) el.textContent = `v${data.version}`;
        })
        .catch(() => {});
}

// --- Staleness detection ---
// Checks the server's version + instance_id against what was stored in sessionStorage.
// If they differ (server restarted or updated), shows a reload banner at the top of the page.
function checkStaleness() {
    fetch('/version')
        .then(r => r.json())
        .then(data => {
            const key = `${data.version}::${data.instance_id}`;
            const stored = sessionStorage.getItem('sanshain_instance');
            if (!stored) {
                // First visit this session — store and move on
                sessionStorage.setItem('sanshain_instance', key);
                return;
            }
            if (stored !== key) {
                showReloadBanner();
            }
        })
        .catch(() => {});
}

function showReloadBanner() {
    if (document.getElementById('sanshain-reload-banner')) return;
    const banner = document.createElement('div');
    banner.id = 'sanshain-reload-banner';
    banner.style.cssText = 'position:fixed;top:0;left:0;right:0;z-index:9999;background:#fef3c7;border-bottom:2px solid #f59e0b;padding:10px 16px;display:flex;align-items:center;justify-content:center;gap:12px;font-size:14px;color:#92400e;font-family:ui-sans-serif,system-ui,sans-serif;';
    banner.innerHTML = `
        <svg style="width:20px;height:20px;flex-shrink:0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M12 2a10 10 0 100 20 10 10 0 000-20z"/>
        </svg>
        <span>The server has been updated or restarted. You may be viewing stale data.</span>
        <button onclick="sessionStorage.setItem('sanshain_instance','');location.reload()" style="background:#f59e0b;color:white;border:none;padding:5px 14px;border-radius:6px;cursor:pointer;font-weight:600;font-size:13px;">Reload</button>
        <button onclick="this.parentElement.remove();sessionStorage.setItem('sanshain_instance','')" style="background:none;border:none;cursor:pointer;color:#92400e;font-size:18px;line-height:1;padding:0 4px;" title="Dismiss">&times;</button>
    `;
    document.body.prepend(banner);
}

// Run staleness check on every page load
checkStaleness();

// --- Password visibility toggle ---
function togglePasswordVisibility(inputId, btn) {
    const input = document.getElementById(inputId);
    if (!input) return;
    if (input.type === 'password') {
        input.type = 'text';
        btn.textContent = 'Hide';
    } else {
        input.type = 'password';
        btn.textContent = 'Show';
    }
}
