// 自绘图标。不用图标库:一是少一个依赖,二是尺寸/线宽能统一到 1.5px 发丝线上。
// 全部 currentColor,不引入颜色。

interface P { size?: number }

const S = (size: number) => ({
  width: size, height: size, viewBox: "0 0 24 24",
  fill: "none", stroke: "currentColor",
  strokeWidth: 1.6, strokeLinecap: "round" as const, strokeLinejoin: "round" as const,
});

export const IconMinus = ({ size = 16 }: P) => (<svg {...S(size)}><path d="M5 12h14" /></svg>);
export const IconSquare = ({ size = 16 }: P) => (<svg {...S(size)}><rect x="5.5" y="5.5" width="13" height="13" rx="2.5" /></svg>);
export const IconX = ({ size = 16 }: P) => (<svg {...S(size)}><path d="M6 6l12 12M18 6L6 18" /></svg>);
export const IconPlay = ({ size = 16 }: P) => (<svg {...S(size)}><path d="M8 5.5v13l11-6.5z" /></svg>);
export const IconPause = ({ size = 16 }: P) => (<svg {...S(size)}><path d="M9 5.5v13M15 5.5v13" /></svg>);
export const IconStop = ({ size = 16 }: P) => (<svg {...S(size)}><rect x="6.5" y="6.5" width="11" height="11" rx="2" /></svg>);

export const IconStage = ({ size = 16 }: P) => (<svg {...S(size)}><rect x="3.5" y="5" width="17" height="11" rx="2.5" /><path d="M8 19.5h8M12 16v3.5" /></svg>);
export const IconFolder = ({ size = 16 }: P) => (<svg {...S(size)}><path d="M3.5 7.5A2 2 0 015.5 5.5h3.2l1.6 2h8.2a2 2 0 012 2v7a2 2 0 01-2 2h-13a2 2 0 01-2-2z" /></svg>);
export const IconFile = ({ size = 16 }: P) => (<svg {...S(size)}><path d="M6 3.5h7l5 5v12a1 1 0 01-1 1H6a1 1 0 01-1-1v-16a1 1 0 011-1z" /><path d="M13 3.5v5h5" /></svg>);
export const IconNote = ({ size = 16 }: P) => (<svg {...S(size)}><path d="M5 4.5h14v11l-4 4H5z" /><path d="M15 19.5v-4h4" /><path d="M8.5 9h7M8.5 12.5h4" /></svg>);
export const IconFilm = ({ size = 16 }: P) => (<svg {...S(size)}><rect x="3.5" y="5" width="17" height="14" rx="2.5" /><path d="M8 5v14M16 5v14M3.5 12h17" /></svg>);
export const IconGear = ({ size = 16 }: P) => (<svg {...S(size)}><circle cx="12" cy="12" r="3" /><path d="M12 3.5v2.2M12 18.3v2.2M4.9 7.6l1.9 1.1M17.2 15.3l1.9 1.1M4.9 16.4l1.9-1.1M17.2 8.7l1.9-1.1" /></svg>);
export const IconInfo = ({ size = 16 }: P) => (<svg {...S(size)}><circle cx="12" cy="12" r="8.5" /><path d="M12 11v5.5M12 7.8v.4" /></svg>);

export const IconChevronR = ({ size = 16 }: P) => (<svg {...S(size)}><path d="M9.5 6l6 6-6 6" /></svg>);
export const IconChevronD = ({ size = 16 }: P) => (<svg {...S(size)}><path d="M6 9.5l6 6 6-6" /></svg>);
export const IconLock = ({ size = 16 }: P) => (<svg {...S(size)}><rect x="5" y="10.5" width="14" height="9" rx="2.5" /><path d="M8.5 10.5V8a3.5 3.5 0 017 0v2.5" /></svg>);
export const IconArrowDown = ({ size = 16 }: P) => (<svg {...S(size)}><path d="M12 5.5v13M6.5 13l5.5 5.5L17.5 13" /></svg>);
export const IconTrash = ({ size = 16 }: P) => (<svg {...S(size)}><path d="M4.5 7h15M9.5 7V5h5v2M6.5 7l1 12.5h9L17.5 7" /></svg>);
export const IconExport = ({ size = 16 }: P) => (<svg {...S(size)}><path d="M12 15.5V4M8 8l4-4 4 4" /><path d="M5 15v3.5a1 1 0 001 1h12a1 1 0 001-1V15" /></svg>);
export const IconShield = ({ size = 16 }: P) => (<svg {...S(size)}><path d="M12 3.5l7 2.5v6c0 4.2-2.9 7.3-7 8.5-4.1-1.2-7-4.3-7-8.5V6z" /><path d="M9.5 12l2 2 3.5-4" /></svg>);
export const IconWarn = ({ size = 16 }: P) => (<svg {...S(size)}><path d="M12 4.5l8.5 15h-17z" /><path d="M12 10v4.2M12 16.6v.3" /></svg>);
export const IconSkill = ({ size = 16 }: P) => (<svg {...S(size)}><path d="M12 4l2.2 5.3L20 11l-5.8 1.7L12 18l-2.2-5.3L4 11l5.8-1.7z" /></svg>);
export const IconBack = ({ size = 16 }: P) => (<svg {...S(size)}><path d="M15 6l-6 6 6 6" /></svg>);
export const IconSidebar = ({ size = 16 }: P) => (<svg {...S(size)}><rect x="3.5" y="5" width="17" height="14" rx="2.5" /><path d="M9.5 5v14" /></svg>);
