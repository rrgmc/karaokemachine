# `ports/`

**The native application shells**, one directory per platform per product: `ports/machine/android/`, `ports/machine/tizen/`,
`ports/remote/android/`, `ports/remote/ios/`. Gradle, Xcode and .NET projects and nothing else.

**`tools/port/` mirrors it exactly**, so the answer to "which scripts build this?" is the same path
read twice.
