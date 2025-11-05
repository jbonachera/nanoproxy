# Code style

- Follow rust style guidelines
- Follow hexagonal architecture general guidelines
- Use std::sync::Arc for shared data
- Use AtomicUsize for atomic counters
- Use #[derive(Clone)] for structs that need to be cloned
- NEVER comment code, use readable method & variable names, produce methods with a single responsibility
- Try not to import new crates unless absolutely necessary

# Workflow
- Use the TDD approach, write tests to explicit behavior before writing any implementation code
- Be sure to typecheck when you’re done making a series of code changes
- Prefer running single tests, and not the whole test suite, for performance