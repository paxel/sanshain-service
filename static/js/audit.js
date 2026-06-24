async function loadTimeline() {
    showLoader();
    try {
        const res = await apiCall('/api/audit/timeline?limit=50');
        if (!res.ok) {
            throw new Error('Failed to fetch timeline');
        }
        const data = await res.json();
        renderTimeline(data);
    } catch (err) {
        console.error(err);
        document.getElementById('audit-timeline').innerHTML = `<div class="text-center py-20 text-red-500">Error loading timeline: ${err.message}</div>`;
    } finally {
        hideLoader();
    }
}

function renderTimeline(versions) {
    const container = document.getElementById('audit-timeline');
    if (versions.length === 0) {
        container.innerHTML = '<div class="text-center py-20 text-slate-400">No specification updates found.</div>';
        return;
    }

    container.innerHTML = versions.map((v, index) => {
        const date = new Date(v.created_at);
        const dateStr = date.toLocaleString();
        const apiTypeIcon = v.api_type === 'openapi' ? '🌐' : (v.api_type === 'asyncapi' ? '⚡' : '🔌');
        
        return `
            <div class="timeline-item">
                <div class="bg-white rounded-xl border border-slate-200 shadow-sm overflow-hidden">
                    <div class="p-4 sm:p-6">
                        <div class="flex flex-wrap items-start justify-between gap-4 mb-4">
                            <div>
                                <div class="flex items-center gap-2 mb-1">
                                    <span class="font-bold text-lg text-slate-800">${escapeHtml(v.service_name)}</span>
                                    <span class="px-2 py-0.5 bg-slate-100 text-slate-600 rounded text-xs font-mono">${escapeHtml(v.branch_name)}</span>
                                </div>
                                <div class="text-sm text-slate-500 flex items-center gap-2">
                                    ${apiTypeIcon} <span class="font-medium">${escapeHtml(v.method)}</span> ${escapeHtml(v.path)}
                                </div>
                            </div>
                            <div class="text-right">
                                <div class="text-sm font-medium text-slate-700">${dateStr}</div>
                                <div class="text-xs text-slate-400">Version ${v.version} ${v.username ? `by ${escapeHtml(v.username)}` : ''}</div>
                            </div>
                        </div>
                        
                        ${v.diff_from_previous ? `
                            <div class="mt-4">
                                <button onclick="toggleDiff(${v.id})" class="text-indigo-600 hover:text-indigo-800 text-sm font-medium flex items-center gap-1">
                                    <svg id="diff-icon-${v.id}" class="w-4 h-4 transition-transform" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"></path>
                                    </svg>
                                    View Changes
                                </button>
                                <div id="diff-container-${v.id}" class="hidden mt-4 border border-slate-100 rounded-lg overflow-hidden bg-slate-50">
                                    <div id="diff-content-${v.id}" class="diff-view"></div>
                                </div>
                            </div>
                        ` : `
                            <div class="mt-4 text-xs text-slate-400 italic">Initial version or no diff available.</div>
                        `}
                    </div>
                </div>
            </div>
        `;
    }).join('');

    // Pre-parse diffs if they are small or just initialize them
    versions.forEach(v => {
        if (v.diff_from_previous) {
            // We'll render on demand to keep initial load fast
        }
    });
}

function toggleDiff(id) {
    const container = document.getElementById(`diff-container-${id}`);
    const icon = document.getElementById(`diff-icon-${id}`);
    const content = document.getElementById(`diff-content-${id}`);
    
    const isHidden = container.classList.contains('hidden');
    
    if (isHidden) {
        container.classList.remove('hidden');
        icon.style.transform = 'rotate(180deg)';
        
        // Render diff if not already rendered
        if (content.innerHTML === '') {
            renderDiffContent(id);
        }
    } else {
        container.classList.add('hidden');
        icon.style.transform = 'rotate(0deg)';
    }
}

async function renderDiffContent(id) {
    // Find the version in our data (we could also fetch it again if needed, but it's in the timeline)
    // Actually, we need the diff text.
    // I'll re-fetch the specific version history if I don't have it, but for now I'll assume it's in the data.
    // Wait, I need a way to access the data from the toggleDiff function.
    // I'll store the fetched data globally for simplicity in this demo.
}

// Update loadTimeline to store data
let timelineData = [];
const originalLoadTimeline = loadTimeline;
loadTimeline = async function() {
    showLoader();
    try {
        const res = await apiCall('/api/audit/timeline?limit=50');
        if (!res.ok) throw new Error('Failed to fetch timeline');
        timelineData = await res.json();
        renderTimeline(timelineData);
    } catch (err) {
        console.error(err);
        document.getElementById('audit-timeline').innerHTML = `<div class="text-center py-20 text-red-500">Error loading timeline: ${err.message}</div>`;
    } finally {
        hideLoader();
    }
};

function renderDiffContent(id) {
    const version = timelineData.find(v => v.id === id);
    if (!version || !version.diff_from_previous) return;

    const diffContent = document.getElementById(`diff-content-${id}`);
    
    try {
        const diffHtml = Diff2Html.html(version.diff_from_previous, {
            drawFileList: false,
            matching: 'lines',
            outputFormat: 'side-by-side',
            renderNothingWhenEmpty: false,
        });
        diffContent.innerHTML = diffHtml;
    } catch (err) {
        console.error('Diff rendering failed:', err);
        diffContent.innerHTML = '<div class="p-4 text-red-500 text-sm">Failed to render diff.</div>';
    }
}
