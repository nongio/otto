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

// https://tj.ie/building-a-table-of-contents-with-the-intersection-observer-api/
// TODO(bfirsh): this could be improved with a scroll handler.
// It should probably highlight a section if it is _mostly_ visible on a page, with some special cases around parent sections. This is basically impossible to do with visibility API.

function highlightFirstActive() {
    document.querySelectorAll("nav li").forEach(link => {
        link.classList.remove('active')
    })

    let firstVisibleLink = document.querySelector('nav li.visible');
    if (firstVisibleLink) {
        let firstVisibleChild = firstVisibleLink.querySelector("li.visible");
        if (firstVisibleChild) {
            firstVisibleChild.classList.add('active')
        } else {
            firstVisibleLink.classList.add('active')
        }
    }
}

function startNavObservation() {

    const observer = new IntersectionObserver(entries => {
        entries.forEach(entry => {
            const id = entry.target.getAttribute('id');
            const link = document.querySelector(`nav li a[href="#${id}"]`);
            if (link) {
                if (entry.intersectionRatio > 0) {
                    link.parentElement.classList.add('visible');
                } else {
                    link.parentElement.classList.remove('visible');
                }
            }
        });
        highlightFirstActive();
    });

    // Track all sections that have an `id` applied
    document.querySelectorAll('section[id]').forEach((section) => {
        observer.observe(section);
    });

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
