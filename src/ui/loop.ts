// Live-binding bridge for the render loop during extraction. main.ts registers the
// real functions at boot; extracted modules import { render }/{ refresh } from here.
// Folded into ui/render.ts in Task 6.
export let render: () => void = () => {};
export let refresh: () => void = () => {};
export function setLoop(r: () => void, rf: () => void): void {
  render = r;
  refresh = rf;
}
