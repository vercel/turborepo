# React + TypeScript + Vite

This app provides a minimal setup to get React working in Vite with HMR. It is served by the microfrontends proxy under `/admin`.

It uses [@vitejs/plugin-react-swc](https://github.com/vitejs/vite-plugin-react/blob/main/packages/plugin-react-swc), which uses [SWC](https://swc.rs/) for Fast Refresh.

## ESLint configuration

Linting uses the shared flat config from `@repo/eslint-config/vite` (see `eslint.config.js`), which combines `typescript-eslint`, `eslint-plugin-react-hooks`, and `eslint-plugin-react-refresh`.

If you are developing a production application, consider enabling type-aware lint rules by extending `tseslint.configs.recommendedTypeChecked` (or `strictTypeChecked`) and setting `languageOptions.parserOptions.projectService: true`.
