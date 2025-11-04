# Code style

- Follow rust style guidelines
- Follow hexagonal architecture general guidelines
- Use std::sync::Arc for shared data
- Use AtomicUsize for atomic counters
- Use #[derive(Clone)] for structs that need to be cloned
- Don't comment code, use readable method & variable names
- Try not to import new crates unless absolutely necessary

# Workflow
- Be sure to typecheck when you’re done making a series of code changes
- Prefer running single tests, and not the whole test suite, for performance