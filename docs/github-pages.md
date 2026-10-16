# Maintaining the GitHub Pages site

The public introduction is in [index.html](index.html). It uses GitHub Pages' Jekyll
support, a custom layout in `_layouts/default.html`, and the stylesheet in
`assets/site.css`. No JavaScript, external fonts, or frontend build is required. The technical references remain ordinary Markdown
documents in this directory.

## Enable publishing

In the repository on GitHub, open **Settings → Pages**, select **Deploy from a
branch**, choose the branch containing these files and the **/docs** directory,
and save. The expected address is
<https://pezia.github.io/rocc-server-emulator/>. This change prepares the files;
it does not enable publishing in the repository settings.

If the repository is renamed or moved, update `url`, `baseurl`, and `repository`
in `_config.yml`, and the repository links in `index.html`.

## Add your photographs and screenshots

Place the owner-supplied files in `docs/media/` using these names, or update the
paths in `_data/gallery.yml` to match your filenames:

| File | Content |
| --- | --- |
| `DSCF3240.JPG` | Cabinet front; also used in the homepage hero. |
| `DSCF3229.JPG` | CRT terminal. |
| `DSCF3219.JPG` | Keyboard. |
| `DSCF3244.JPG` | Cabinet identification plate. |
| `DSCF3249.JPG` | Internal boards, ribbon cables, and connector panel. |
| `screenshot_after_boot.png` | WMS4 main menu after boot. |
| `screenshot_system_customization.png` | System configuration and software base level. |
| `screenshot_dynamic_memory_evaluator.png` | Dynamic Core Evaluator memory diagnostics. |
| `screenshot_customer_program.png` | Hungarian-language customer application menu. |

For each image, edit its title, descriptive alt text, and caption in
`_data/gallery.yml`. Give photographs a credit and identify the model only when
confirmed. For screenshots, describe what operation the screen demonstrates;
include the system name, release, and capture date when known. Remove private information before
adding captures to the public repository.

Missing files render as designed, labeled placeholders rather than broken images.
The homepage hero uses `DSCF3240.JPG`, with a decorative terminal illustration
as a fallback if that photograph is absent. The five supplied photographs are
listed in the machine gallery with descriptive captions. When a file is added,
Jekyll automatically displays it with its caption on the next build. Review and
replace any provisional caption at the same time. Add more entries to the
appropriate gallery list for additional images.

Once the galleries are populated, update the introductory text above them in
`index.html` to describe the supplied material rather than future additions.
The gallery retains each image’s natural proportions inside its frame and
links to the full-size original. Update the hero alt text in `index.html` if
you choose a different view of the machine.

## Check the site

Review the GitHub Pages build result after publishing. Check the site at its
repository subpath, confirm that each image opens at full size, read its caption,
and follow the historical and technical links. Markdown viewed directly on
GitHub does not render the Liquid gallery includes; view the built Pages site
to check the galleries.
