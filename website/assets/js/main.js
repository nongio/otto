window.addEventListener('DOMContentLoaded', () => {

    convertToNestedSections(document.querySelector('main'));
    addParentHeadingAttribute();
    startNavObservation();
    initThemeToggle();
    initBarClock();
    initCodeBlocks();

});

var SHELL_LANGS = ['sh', 'bash', 'shell', 'zsh', 'console'];

// Solar "copy" and "copy-check", linear (@solar-icons/static 2.3.2).
var SOLAR_COPY_PATHS =
    '<path d="M6 11C6 8.17157 6 6.75736 6.87868 5.87868C7.75736 5 9.17157 5 12 5H15C17.8284 5 19.2426 5 20.1213 5.87868C21 6.75736 21 8.17157 21 11V16C21 18.8284 21 20.2426 20.1213 21.1213C19.2426 22 17.8284 22 15 22H12C9.17157 22 7.75736 22 6.87868 21.1213C6 20.2426 6 18.8284 6 16V11Z"/>' +
    '<path d="M6 19C4.34315 19 3 17.6569 3 16V10C3 6.22876 3 4.34315 4.17157 3.17157C5.34315 2 7.22876 2 11 2H15C16.6569 2 18 3.34315 18 5"/>';
var SOLAR_SVG = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">';
var COPY_ICON = SOLAR_SVG + SOLAR_COPY_PATHS + '</svg>';
var COPIED_ICON = SOLAR_SVG + SOLAR_COPY_PATHS + '<path d="M10.25 13.9875L12.1071 15.9375L16.75 11.0625"/></svg>';

// Every code block gets a terminal bar with its name, and a copy button on
// the first line of whatever the pointer is over. A shell command copies
// whole, with its continuation lines and heredoc; a script copies whole from
// any line; anything else copies the hovered line.
function initCodeBlocks() {
    document.querySelectorAll('main pre > code').forEach(code => {
        var pre = code.parentElement;
        var lang = (code.dataset.lang || '').toLowerCase();
        var shell = SHELL_LANGS.indexOf(lang) !== -1;

        var bar = document.createElement('span');
        bar.className = 'term-bar';
        var title = document.createElement('span');
        title.className = 'term-title';
        title.textContent = shell ? 'Terminal' : lang;
        bar.append(title);
        pre.prepend(bar);

        splitLines(code);
        var groupOf = new Map();
        lineGroups(code, shell).forEach(group => group.forEach(line => groupOf.set(line, group)));

        var copy = document.createElement('button');
        copy.className = 'term-copy';
        copy.type = 'button';
        copy.setAttribute('aria-label', 'Copy');
        copy.title = 'Copy';
        copy.innerHTML = COPY_ICON;
        pre.append(copy);

        var current = null;
        function show(group) {
            if (group === current) return;
            if (current) current.forEach(l => l.classList.remove('term-hover'));
            current = group;
            copy.classList.toggle('shown', !!group);
            if (!group) return;
            group.forEach(l => l.classList.add('term-hover'));
            // One button per command, on its first line and never taller
            // than a line.
            var box = group[0].getBoundingClientRect();
            copy.style.top = (box.top - pre.getBoundingClientRect().top - pre.clientTop) + 'px';
            copy.style.height = copy.style.width = box.height + 'px';
            copy.innerHTML = COPY_ICON;
        }

        code.addEventListener('mouseover', e => {
            var line = e.target.closest('code > span');
            if (line && line.parentElement === code) show(groupOf.get(line) || null);
        });
        pre.addEventListener('mouseleave', () => show(null));

        function copyCurrent() {
            if (!current) return;
            // textContent, not innerText: the highlighter's lines are flex
            // boxes, which innerText would double-space, and the drawn
            // prompt stays out of it.
            var text = current.map(l => l.textContent).join('').replace(/\n+$/, '');
            navigator.clipboard.writeText(text).then(() => { copy.innerHTML = COPIED_ICON; });
        }
        copy.addEventListener('click', copyCurrent);
        // A click on the line copies it too; a drag that selects text does not.
        code.addEventListener('click', () => {
            if (window.getSelection().isCollapsed) copyCurrent();
        });
    });
}

// The highlighter writes one child span per line; a block it left alone
// (no language) is plain text, so give it the same shape.
function splitLines(code) {
    if (code.children.length) return;
    var text = code.textContent;
    code.textContent = '';
    text.split(/(?<=\n)/).forEach(part => {
        var line = document.createElement('span');
        line.style.display = 'block';
        line.textContent = part;
        code.append(line);
    });
}

// The lines one copy takes, per hovered line. Shell blocks also get a
// prompt on each line a reader would type: not on comments, continuation
// lines, heredoc bodies or indented lines, and not at all on a block that
// is a script rather than a list of commands.
function lineGroups(code, shell) {
    var lines = Array.from(code.children);
    var texts = lines.map(line => line.textContent.replace(/\n$/, ''));
    var filled = lines.filter((line, i) => texts[i].trim());
    if (!shell) return filled.map(line => [line]);
    var script = texts.some(t => /^(#!|(while|for|if|case|until|function)\b|\w+\(\)\s*\{)/.test(t));
    if (script) return [filled];

    var groups = [];
    var current = null;
    var heredoc = null;
    var continued = false;
    texts.forEach((text, i) => {
        var line = lines[i];
        if (heredoc) {
            if (current) current.push(line);
            if (text.trim() === heredoc) heredoc = null;
            continued = false;
            return;
        }
        if (/^\s*#/.test(text)) {
            line.classList.add('term-comment');
            current = null;
        } else if (continued && current) {
            current.push(line);
        } else if (/^\S/.test(text)) {
            line.classList.add('term-cmd');
            current = [line];
            groups.push(current);
        } else {
            current = null;
        }
        var here = text.match(/<<-?\s*['"]?(\w+)['"]?/);
        if (here) heredoc = here[1];
        continued = /\\$/.test(text);
    });
    return groups;
}

// The top bar's clock, written the way Otto's bar writes it: the weekday,
// the date and the time, in the reader's own locale.
function initBarClock() {
    var clock = document.querySelector('#bar-clock');
    if (!clock) return;
    var date = new Intl.DateTimeFormat(undefined, { weekday: 'long', day: 'numeric', month: 'long' });
    var time = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' });

    function tick() {
        var now = new Date();
        clock.textContent = date.format(now) + '  ' + time.format(now);
        clock.dateTime = now.toISOString();
        // Wake on the next minute, not on a fixed interval that drifts.
        setTimeout(tick, 60000 - (now.getSeconds() * 1000 + now.getMilliseconds()) + 50);
    }
    tick();
}

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
        // The label names the theme a click switches to.
        var next = currentTheme() === 'dark' ? 'light' : 'dark';
        button.setAttribute('aria-label', 'Switch to ' + next + ' mode');
        button.title = 'Switch to ' + next + ' mode';
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
