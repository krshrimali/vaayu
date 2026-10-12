# Thoughts on HTML previews

Yes, we could add HTML previews to Vaayu. The main choice is whether we want to
read the page's content or see the page exactly as it looks in a browser.

Vaayu already highlights HTML source. Its full-screen and split Markdown previews
give us useful pieces to reuse, but those previews currently accept Markdown
files only. HTML needs its own rendering path.

There are three useful options:

1. **Read HTML inside the terminal.** Show headings, paragraphs, lists, links,
   tables, and code as readable terminal text. This could work without terminal
   graphics support and would be useful for documentation. It would simplify the
   page's appearance: CSS layouts, animations, and JavaScript-driven content
   would need a browser. We would need an HTML parser and rules for turning its
   content into our existing styled rows.

2. **Open a real browser from Vaayu.** Add a command that opens the current HTML
   file in the user's browser. The browser handles the page's styling, images,
   scripts, and interactive controls. We could add automatic reload when the
   file is saved. Some projects would need a local web server or their existing
   development server, especially for scripts and application pages. This is
   the most practical first step for an accurate preview, and it avoids terminal
   graphics limits on Windows and Linux.

3. **Show a picture of the real page inside Vaayu.** Use a browser running in the
   background to take a screenshot, then display that image in a preview pane.
   Browser tools already support page screenshots; see the
   [Playwright screenshot documentation](https://playwright.dev/docs/screenshots).
   We could reuse our terminal image display, zoom, and clipping work. This would
   need an installed browser and more work to keep updates fast. The screenshot
   would show the page's appearance, but buttons and forms in that image would
   not be interactive. Windows Terminal would also need the Sixel display
   backend discussed earlier; our current image backend uses Kitty graphics.

I would start with opening the page in a browser, then add reload on save. That
gives us accurate HTML previews with a fairly small addition to the editor.
For this first version, the browser would show saved content. Previewing unsaved
edits could come later, with care to keep relative stylesheet and image paths
working.

A text preview would be a useful next step for people who mainly read HTML
documents. An image preview inside the editor would be worth exploring if seeing
the page beside its source is especially important.

Before choosing an implementation, I would try our own
[TECHNICAL_DECISIONS.html](TECHNICAL_DECISIONS.html), a page with separate CSS and
images, and a page with a JavaScript button. Those examples would help us check
readability, linked files, and the difference between a screenshot and a working
page. Browser launch and reload should be checked on both Windows and Linux.

These are feasibility thoughts; HTML preview support has not been implemented.
