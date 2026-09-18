# `ports/`

**The native application shells**, one directory per platform per product: `ports/machine/android/`,
`ports/remote/android/`, `ports/remote/ios/`. Gradle and Xcode projects and nothing else.

**`tools/port/` mirrors it exactly**, so the answer to "which scripts build this?" is the same path
read twice.
