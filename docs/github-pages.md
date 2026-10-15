# Maintaining the GitHub Pages site

The public introduction is in [index.md](index.md). It uses GitHub Pages' Jekyll
support and the Primer theme. The technical references remain ordinary Markdown
documents in this directory.

## Enable publishing

In the repository on GitHub, open **Settings → Pages**, select **Deploy from a
branch**, choose the branch containing these files and the **/docs** directory,
and save. The expected address is
<https://pezia.github.io/rocc-server-emulator/>. This change prepares the files;
it does not enable publishing in the repository settings.

If the repository is renamed or moved, update `url`, `baseurl`, and `repository`
in `_config.yml`, and the repository links in `index.md`.

## Add your photographs and screenshots

Place the owner-supplied files in `docs/media/` using these names, or update the
paths in `_data/gallery.yml` to match your filenames:

| File | Content |
| --- | --- |
| `machine-front.jpg` | Photograph of the physical machine. |
| `machine-terminal.jpg` | Photograph of its terminal and keyboard. |
| `system-startup.png` | Actual startup output from a real guest system in the emulator. |
| `system-session.png` | Actual interactive operation of that guest system. |

For each image, edit its title, descriptive alt text, and caption in
`_data/gallery.yml`. Give photographs a credit and identify the model only when
confirmed. For screenshots, include the system name and release, capture date,
and what operation the screen demonstrates. Remove private information before
adding captures to the public repository.

Missing files render as text placeholders rather than broken images. When a
file is added, Jekyll automatically displays it with its caption on the next
build. Review and replace the provisional caption at the same time. Add more
entries to the appropriate gallery list for additional images.

Once the galleries are populated, update the introductory text above them in
`index.md` to describe the supplied material rather than future additions.

## Check the site

Review the GitHub Pages build result after publishing. Check the site at its
repository subpath, confirm that each image opens at full size, read its caption,
and follow the historical and technical links. Markdown viewed directly on
GitHub does not render the Liquid gallery includes; view the built Pages site
to check the galleries.
