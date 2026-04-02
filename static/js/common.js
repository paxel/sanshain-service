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
