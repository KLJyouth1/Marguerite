import "@testing-library/jest-dom";

// happy-dom 不对齐 matchMedia；个别组件/测试暂不需要。
// 预留：若未来接入主题切换的媒体查询断言，可在此补齐 mock。
if (!window.matchMedia) {
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }),
  });
}
