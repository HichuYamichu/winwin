# winwin

winwin is a window manager for Windows inspired by my experience using xMonad. Currently this project is in its alpha state, hovewer it's stable enough to play around with it for a bit.

# Installation and configuration

winwin uses the popular WM config scheme, which is: Your config is a program that controls what happens. This gives the user maximum control and customization options at the cost of much greater difficulty with setting it up. Rust compile times make it a really bad candidate for this type of work, but once you're happy with your config, you don't need to bother with it again.

See main.rs file for configuration example showcasing most currently available features: [main.rs](https://github.com/HichuYamichu/winwin/blob/master/winwin/src/main.rs)

# Rough edges

- Most window manipulation functions are not good enough; there are small but noticable gaps between windows after applying the given layout.
- Window ordering can change unpredictably when window is open or closed.
- There is no virtual desktop support yet (the goal is to use windows native virtual desktops instead of implementing a custom solution)..
- There is a huge amount of window positioning functionality that has not yet been implemented.
- Once EventQueue is created, it MUST be pulled for events; not doing so will cause internal quque to fill up, which in turn can cause most desktop applications to stop responding.


# Performance

winwin in made with performance in mind. Its event-based architecture and careful memory management guarantee a low CPU and memory footprint.
