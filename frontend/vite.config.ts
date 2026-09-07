import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  // Set BASE_PATH when the site is served under a sub-path (GitHub Pages: /Intendancy/).
  base: process.env.BASE_PATH || '/',
  plugins: [react(), tailwindcss()],
})
