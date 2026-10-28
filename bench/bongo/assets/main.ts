import { on } from 'bongo/client';

// client() is mounted via <bongo:client transitions prefetch /> in html.hbs.
on('bongo:navigated', ({ to }) => {
	console.log('[bongo] navigated to', to.pathname);
});

// Back-to-top button: appears after scrolling down 300px.
let btn = document.createElement('button');
btn.id = 'back-to-top';
btn.textContent = '↑ Top';
btn.setAttribute('aria-label', 'Back to top');
document.body.appendChild(btn);

btn.addEventListener('click', () => {
	window.scrollTo({ top: 0, behavior: 'smooth' });
});

let visible = false;

window.addEventListener('scroll', () => {
	let shouldShow = window.scrollY > 300;
	if (shouldShow !== visible) {
		visible = shouldShow;
		btn.classList.toggle('visible', visible);
	}
}, { passive: true });
