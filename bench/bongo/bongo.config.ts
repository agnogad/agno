import { markdown } from '@bongo/markdown';
import { shiki } from '@bongo/markdown/highlight';
import type { Config } from 'bongo';

export default {
	$site: {
		lang: 'en',
		title: 'My Website',
		url: 'https://example.com',
		description: 'A site built with bongo.',
	},

	$utils: {
		year(): number {
			return new Date().getFullYear();
		},
		humanDate(iso: string): string {
			let d = new Date(iso + 'T12:00:00Z');
			return d.toLocaleDateString('en-US', {
				year: 'numeric',
				month: 'long',
				day: 'numeric',
				timeZone: 'UTC',
			});
		},
		sortByDate<T extends { params: { date?: unknown } }>(pages: T[]): T[] {
			return [ ...pages ].sort((a, b) => {
				let ad = typeof a.params.date === 'string' ? a.params.date : '';
				let bd = typeof b.params.date === 'string' ? b.params.date : '';
				return bd.localeCompare(ad); // newest first
			});
		},
	},

	plugins: [
		markdown({
			highlighter: shiki({
				theme: 'github-dark',
				langs: ['bash', 'css', 'hbs', 'js', 'rust', 'ts'],
			}),
		}),
	],
} satisfies Config
