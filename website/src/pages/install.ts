export const prerender = false;

export async function GET() {
  return new Response(null, {
    status: 302,
    headers: {
      Location:
        "https://raw.githubusercontent.com/ianjamesburke/PLEXI/main/install.sh",
    },
  });
}
