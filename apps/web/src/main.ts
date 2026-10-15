import './app.css';
import App from './App.svelte';
import { mount } from 'svelte';
import { applyDocumentLanguage } from './lib/document-language';

const target = document.getElementById('app');

if (!target) {
  throw new Error('Unable to find the application mount point.');
}

// index.html ships lang="en" for first paint; this keeps it truthful if a
// translated shell is ever added (see lib/document-language.ts).
applyDocumentLanguage();
window.addEventListener('languagechange', () => applyDocumentLanguage());

mount(App, { target });

if ('serviceWorker' in navigator && import.meta.env.PROD) {
  window.addEventListener('load', () => {
    void navigator.serviceWorker.register('/sw.js', { scope: '/' });
  });
}
