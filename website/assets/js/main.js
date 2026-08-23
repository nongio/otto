window.addEventListener('DOMContentLoaded', () => {

    convertToNestedSections(document.querySelector('main'));
    addParentHeadingAttribute();
    startNavObservation();
    initThemeToggle();

});

// The stored override (if any) is already applied to <html> by the
// inline script in <head>, before first paint. This just wires up the
// button and keeps its label in sync.
function initThemeToggle() {
    var button = document.querySelector('#theme-toggle');
    var systemDark = window.matchMedia('(prefers-color-scheme: dark)');

    function currentTheme() {
        var stored = localStorage.getItem('otto-theme');
        if (stored === 'dark' || stored === 'light') return stored;
        return systemDark.matches ? 'dark' : 'light';
    }

    function render() {
        // Label is the theme a click will switch TO.
        button.textContent = currentTheme() === 'dark' ? 'Light' : 'Dark';
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
