import bootstrap from '../../../scripts/install-windows.ps1?raw';

export const prerender = false;
export async function GET() {
  return new Response(bootstrap, {
    headers: { 'Content-Type': 'text/plain; charset=utf-8', 'Cache-Control': 'no-cache' },
  });
}
