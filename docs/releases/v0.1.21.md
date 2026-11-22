## Faster

- **Long runs no longer slow down as they go.** Before each recording, Studio counted the progress of the whole project again. On a project with many thousands of recordings that count took longer and longer, and short clips (for example from live stations) sent the window a message each. Studio now counts at most once a second and updates the window at most four times a second.
- **The Review species list stays quick after Perch.** Perch can find thousands of species. The list now shows the first 150 and adds more as you scroll, so each answer, clip and key press no longer redraws the whole list. Search still looks through every species.
- **Camera-trap photos page back instantly.** Grid pictures already made are kept while Studio is open, so going back a page or reopening a species does not read the full photos again.
- **Spectrograms are drawn a little faster,** most noticeably for long live-station clips.
- **Big projects open their recorder list faster, and removing recorders or processing recordings again is faster when a second reviewer's answers are loaded.** The first time a project is opened with this version, Studio adds a few small lookup indexes to it, which can take a few seconds on a very large project.
