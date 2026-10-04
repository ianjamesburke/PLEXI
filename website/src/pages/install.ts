import bootstrap from '../../../install.sh?raw';

export const prerender = false;
export async function GET() {
  return new Response(bootstrap, {
    headers: { 'Content-Type': 'text/x-shellscript; charset=utf-8', 'Cache-Control': 'no-cache' },
  });
}
