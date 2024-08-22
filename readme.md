# winwin

winwin is a window manager for Windows inspired by my experience using xMonad. Currently this project is in its alpha state.

# Installation and configuration

winwin uses the popular WM config scheme, which is: Your config is a program that controls what happens. This gives the user maximum control and customization options at the cost of much greater difficulty with setting it up. Rust compile times make it a really bad candidate for this type of work, but once you're happy with your config, you don't need to bother with it again.

See main.rs file for configuration example showcasing most currently available features: [main.rs](https://github.com/HichuYamichu/winwin/blob/master/winwin/src/main.rs)

# Rough edges

- There is no virtual desktop support yet (the goal is to use windows native virtual desktops instead of implementing a custom solution).
- There is a huge amount of window positioning functionality that has not yet been implemented.
- Once EventQueue is created, it MUST be pulled for events; not doing so will cause internal quque to fill up, which in turn can cause most desktop applications to stop responding.

All of the above issues are going to be resolved soon.

# Design considerations

- winwin does not use any major 3rd party libraries; everything is handmade.
- winwin uses allocator api wherever possible. Most importantly, it uses an arena allocator for event-scoped allocations.
- winwin uses a custom IPC and event loop solution, ensuring optimal thread utilization. There are no spin loops; IO is done with IOCP in order to avoid wasted CPU cycles.
- winwin's api is mostly procedural with a functional feel. This makes it easy for rust newbies to use and configure.
- winwin functions do not return errors. The only failure points in this library are Windows calls. Their errors are not actionable for the end user in almost all cases. Instead of propagating errors up, winwin attempts to perform requested actions and logs any failure cases. winwin keeps a minimal internal state and all changes to it always succeed and are always valid; failing actions do not leave the application in an invalid state.


