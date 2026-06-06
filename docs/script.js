const toggle = document.querySelector('.nav-toggle');
const links = document.querySelector('.nav-links');

if (toggle && links) {
  toggle.addEventListener('click', () => {
    const open = links.classList.toggle('open');
    toggle.setAttribute('aria-expanded', String(open));
  });

  links.querySelectorAll('a').forEach((link) => {
    link.addEventListener('click', () => {
      links.classList.remove('open');
      toggle.setAttribute('aria-expanded', 'false');
    });
  });
}

document.querySelectorAll('.copy-button').forEach((button) => {
  button.addEventListener('click', async () => {
    const text = button.getAttribute('data-copy') || '';
    try {
      await navigator.clipboard.writeText(text);
      const original = button.textContent;
      button.textContent = '已複製';
      setTimeout(() => { button.textContent = original; }, 1400);
    } catch {
      button.textContent = '請手動複製';
    }
  });
});
