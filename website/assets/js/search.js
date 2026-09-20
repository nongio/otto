// Search across the user and developer guides.
//
// The index is Pagefind's (build-search.sh), which reads the finished HTML
// and splits it into chunks the browser fetches only as a query needs them —
// a search costs tens of KB, not the whole corpus. What is written here is
// the card it is shown in: the desktop's launcher, on a web page.
//
// Pagefind's own UI is deliberately not used. Its results, though, are: a
// hit carries sub-results, one per heading, so a result lands on the section
// that answers the question rather than at the top of a long page.

var MAX_RESULTS = 8;
var MAX_SECTIONS = 3;

var script = document.currentScript || document.querySelector('script[data-bundle]');
var bundleUrl = (script && script.dataset.bundle) || '/pagefind/pagefind.js';
var baseUrl = (script && script.dataset.base) || '/';

var dialog = document.querySelector('#search-dialog');
var button = document.querySelector('#search-button');
var input = document.querySelector('#search-input');
var status = document.querySelector('#search-status');
var list = document.querySelector('#search-results');

var pagefind = null;
var loading = null;
var selected = 0;
var generation = 0;

function load() {
    if (loading) return loading;

    loading = import(bundleUrl)
        .then(function (module) {
            // Pagefind stores root-relative URLs, so a site served from a
            // subpath has to say so.
            return module.options({ baseUrl: baseUrl }).then(function () {
                pagefind = module;
            });
        })
        .catch(function (err) {
            loading = null;
            status.textContent = 'Search is unavailable: ' + err.message;
            throw err;
        });

    return loading;
}

// One row per result: the page itself, or the headings inside it that
// matched. The page leads and the heading is the line under it — where you
// are going, then where in it. Both come from Pagefind as an excerpt with
// <mark> around the terms it matched, including ones it stemmed to, which is
// why the highlighting is its job and not ours.
function rows(results) {
    var out = [];

    results.forEach(function (result) {
        var page = (result.meta && result.meta.title) || result.url;
        var sections = (result.sub_results || []).filter(function (sub) {
            return sub.anchor;
        });

        if (!sections.length) {
            out.push({ url: result.url, page: page, section: '', excerpt: result.excerpt });
            return;
        }

        sections.slice(0, MAX_SECTIONS).forEach(function (sub) {
            out.push({
                url: sub.url,
                page: page,
                section: sub.title,
                excerpt: sub.excerpt
            });
        });
    });

    return out;
}

function render(items, query) {
    list.textContent = '';
    selected = 0;

    if (!query) {
        status.textContent = '';
        return;
    }

    if (!items.length) {
        status.textContent = 'Nothing found for “' + query + '”.';
        return;
    }

    status.textContent = items.length + (items.length === 1 ? ' result' : ' results');

    items.forEach(function (item, i) {
        var li = document.createElement('li');
        var link = document.createElement('a');
        link.href = item.url;
        link.id = 'search-result-' + i;

        var page = document.createElement('span');
        page.className = 'result-title';
        page.textContent = item.page;
        link.appendChild(page);

        if (item.section) {
            var section = document.createElement('span');
            section.className = 'result-section';
            section.textContent = item.section;
            link.appendChild(section);
        }

        var excerpt = document.createElement('span');
        excerpt.className = 'result-snippet';
        // Pagefind escapes the text and adds only <mark>.
        excerpt.innerHTML = item.excerpt;
        link.appendChild(excerpt);

        li.appendChild(link);
        list.appendChild(li);
    });

    // The first hit starts selected, as in the desktop launcher: type,
    // press Enter, and you are at the best answer.
    select(0);
}

function links() {
    return list.querySelectorAll('a');
}

function select(index) {
    var all = links();
    if (!all.length) return;

    var clamped = Math.max(0, Math.min(all.length - 1, index));

    all.forEach(function (link) { link.classList.remove('selected'); });
    all[clamped].classList.add('selected');
    all[clamped].scrollIntoView({ block: 'nearest' });
    input.setAttribute('aria-activedescendant', all[clamped].id);
    selected = clamped;
}

function move(delta) {
    var all = links();
    if (!all.length) return;

    // Wraps, so holding Down at the end of a short list returns to the top
    // instead of doing nothing.
    select((selected + delta + all.length) % all.length);
}

function update() {
    var query = input.value.trim();

    if (!query) {
        render([], '');
        return;
    }

    if (!pagefind) {
        status.textContent = 'Loading…';
        load().then(update, function () {});
        return;
    }

    var mine = ++generation;

    pagefind.debouncedSearch(query, {}, 120).then(function (search) {
        // A newer keystroke has already been searched for.
        if (!search || mine !== generation) return;

        return Promise.all(search.results.slice(0, MAX_RESULTS).map(function (result) {
            return result.data();
        })).then(function (results) {
            if (mine !== generation) return;
            render(rows(results), query);
        });
    });
}

function open() {
    load().catch(function () {});
    if (!dialog.open) dialog.showModal();
    document.body.classList.remove('menu-open');
    input.select();
}

button.addEventListener('click', open);
input.addEventListener('input', update);

input.addEventListener('keydown', function (event) {
    if (event.key === 'ArrowDown') {
        event.preventDefault();
        move(1);
    } else if (event.key === 'ArrowUp') {
        event.preventDefault();
        move(-1);
    } else if (event.key === 'Enter') {
        var all = links();
        if (all.length) {
            event.preventDefault();
            all[selected].click();
        }
    }
});

// Clicking a result navigates; without this the dialog stays open over the
// page it just took you to, when the hit is on the page you are already on.
list.addEventListener('click', function () { dialog.close(); });

dialog.addEventListener('click', function (event) {
    if (event.target === dialog) dialog.close();
});

document.addEventListener('keydown', function (event) {
    var typing = /^(input|textarea|select)$/i.test(event.target.tagName) ||
        event.target.isContentEditable;

    // Ctrl+K rather than the desktop launcher's Ctrl+P: in a browser that
    // one is Print, and taking it would cost a page its printing.
    if ((event.key === 'k' || event.key === 'K') && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        open();
    } else if (event.key === '/' && !typing && !dialog.open) {
        event.preventDefault();
        open();
    }
});
