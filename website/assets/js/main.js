window.addEventListener('DOMContentLoaded', () => {

    convertToNestedSections(document.querySelector('main'));
    addParentHeadingAttribute();
    startNavObservation();
    initThemeToggle();
    initComments();

});

function currentTheme() {
    var stored = localStorage.getItem('otto-theme');
    if (stored === 'dark' || stored === 'light') return stored;
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

// Every h2 and h3 section of a page that allows comments gets a Comment
// button. It opens a giscus thread under the heading, one GitHub Discussion
// per section, keyed by the page path and the section's anchor. One thread is
// open at a time: giscus reads its settings from the script that loads it and
// renders into the first .giscus element, so opening another section replaces
// both.
function initComments() {
    var main = document.querySelector('main');
    if (!main || !main.dataset.commentsRepo) return;

    main.querySelectorAll('section > h2, section > h3').forEach(heading => {
        var section = heading.parentElement;
        if (!section.id) return;

        var bar = document.createElement('div');
        bar.className = 'section-comments';
        bar.setAttribute('data-pagefind-ignore', '');
        var button = document.createElement('button');
        button.type = 'button';
        button.className = 'comment-button';
        button.textContent = 'Comment';
        button.addEventListener('click', () => toggleThread(main, bar, section.id));
        bar.appendChild(button);
        heading.after(bar);
    });
}

function toggleThread(main, bar, sectionId) {
    var open = document.querySelector('.giscus');
    var wasHere = open && open.parentElement === bar;
    if (open) {
        open.parentElement.classList.remove('open');
        open.remove();
        document.querySelectorAll('script[src="https://giscus.app/client.js"]')
            .forEach(script => script.remove());
    }
    if (wasHere) return;

    var host = document.createElement('div');
    host.className = 'giscus';
    bar.appendChild(host);
    bar.classList.add('open');

    var script = document.createElement('script');
    script.src = 'https://giscus.app/client.js';
    script.async = true;
    script.crossOrigin = 'anonymous';
    var config = {
        'data-repo': main.dataset.commentsRepo,
        'data-repo-id': main.dataset.commentsRepoId,
        'data-category': main.dataset.commentsCategory,
        'data-category-id': main.dataset.commentsCategoryId,
        'data-mapping': 'specific',
        'data-term': location.pathname.replace(/\/$/, '') + '#' + sectionId,
        'data-strict': '1',
        'data-reactions-enabled': '1',
        'data-emit-metadata': '0',
        'data-input-position': 'top',
        'data-theme': currentTheme(),
        'data-lang': 'en',
        'data-loading': 'lazy',
    };
    Object.keys(config).forEach(key => script.setAttribute(key, config[key]));
    host.after(script);
}

function syncCommentsTheme() {
    var frame = document.querySelector('iframe.giscus-frame');
    if (!frame) return;
    frame.contentWindow.postMessage(
        { giscus: { setConfig: { theme: currentTheme() } } },
        'https://giscus.app'
    );
}

// The stored override (if any) is already applied to <html> by the
// inline script in <head>, before first paint. This just wires up the
// button and keeps its label in sync.
function initThemeToggle() {
    var button = document.querySelector('#theme-toggle');
    var systemDark = window.matchMedia('(prefers-color-scheme: dark)');

    function render() {
        // Label is the theme a click will switch TO.
        button.textContent = currentTheme() === 'dark' ? 'Light' : 'Dark';
        syncCommentsTheme();
    }

    button.addEventListener('click', () => {
        var next = currentTheme() === 'dark' ? 'light' : 'dark';
        localStorage.setItem('otto-theme', next);
        document.documentElement.setAttribute('data-theme', next);
        render();
    });

    systemDark.addEventListener('change', render);
    render();
}

function convertToNestedSections(rootElement) {
    const children = Array.from(rootElement.children);

    children.forEach(element => rootElement.removeChild(element));

    let currentSection = rootElement;
    let currentLevel = 0;

    children.forEach(element => {
        const headingMatch = element.tagName.match(/^h(\d)$/i);

        if (headingMatch) {
            const newLevel = parseInt(headingMatch[1]);

            while (currentLevel + 1 < newLevel) {
                const section = document.createElement('section');
                currentSection.appendChild(section);
                currentSection = section;
                currentLevel++;
            }

            while (currentLevel + 1 > newLevel) {
                currentSection = currentSection.parentNode;
                currentLevel--;
            }

            const id = element.getAttribute('id');

            const newSection = document.createElement('section');
            newSection.setAttribute('id', id);
            element.removeAttribute('id');

            const permalink = document.createElement('a');
            permalink.setAttribute('href', `#${id}`);
            permalink.classList.add('permalink');
            element.appendChild(permalink);

            currentSection.appendChild(newSection);

            currentSection = newSection;
            currentLevel = newLevel;
        }

        currentSection.appendChild(element);
    });
}

function addParentHeadingAttribute() {
    const selector = 'h1,h2,h3,h4,h5,h6';

    document.querySelectorAll(selector).forEach(heading => {
        const parentHeading = heading.parentElement.parentElement.querySelector(selector);

        if (parentHeading) {
            heading.setAttribute('data-parent-heading', parentHeading.textContent);
        }
    });
}

// Highlight the entry for the section you are currently reading.
//
// This is read-only: it observes scroll position and never sets it. Nothing
// here scrolls the page, the contents list, or an element into view — the
// reader is in charge of where the page is and how fast it gets there.
//
// It used to be an IntersectionObserver picking the first section with any
// pixel on screen, which lagged by about a screen: a section you had scrolled
// past kept a sliver visible and stayed highlighted while the next one filled
// the viewport. Instead, track a "reading line" a third of the way down the
// viewport and highlight the last section whose top has crossed it. Sections
// are nested, so the last match is also the deepest one — scrolling into an
// h3 highlights the h3, not its parent h2.
const READING_LINE = 0.33;

function updateActiveNavEntry() {
    const line = window.innerHeight * READING_LINE;
    const sections = document.querySelectorAll('main section[id]');

    let current = null;

    // At the bottom of the page the last section can never cross the line,
    // so nothing down there would ever highlight. Pick it explicitly.
    const atBottom =
        window.innerHeight + window.scrollY >= document.body.scrollHeight - 2;

    if (atBottom) {
        current = sections[sections.length - 1];
    } else {
        sections.forEach(section => {
            if (section.getBoundingClientRect().top <= line) {
                current = section;
            }
        });
    }

    const link = current
        ? document.querySelector(`nav li a[href="#${current.id}"]`)
        : null;
    const active = link && link.parentElement;

    document.querySelectorAll('nav li.active').forEach(li => {
        if (li !== active) li.classList.remove('active');
    });

    if (active) active.classList.add('active');
}

function startNavObservation() {
    let queued = false;

    // Coalesce to one read per frame: a scroll handler that measures on every
    // event is what makes scrolling feel heavy.
    function schedule() {
        if (queued) return;
        queued = true;
        requestAnimationFrame(() => {
            queued = false;
            updateActiveNavEntry();
        });
    }

    window.addEventListener('scroll', schedule, { passive: true });
    window.addEventListener('resize', schedule);
    updateActiveNavEntry();
}

// Script to hide/show menu
var button = document.querySelector('#menu-button');
var menu = document.querySelector('#TableOfContents');
button.addEventListener('click', function (event) {
    document.body.classList.add("menu-open");
});
menu.addEventListener('click', function (event) {
    document.body.classList.remove("menu-open");
});
