//! Slint headless paint bridge.
//!
//! Slint is used strictly as a paint bucket: it renders the Wireframe
//! (`keyboard wireframe.md`) scene into the existing Wayland `wl_shm` ARGB
//! buffer. All layout geometry, hit-testing, gesture state machines and input
//! output stay in Rust (`RenderEngine::calculate_key_rects` is the source of
//! geometry truth).

use std::cell::Cell;
use std::rc::Rc;

use slint::platform::{
    software_renderer::{PremultipliedRgbaColor, SoftwareRenderer},
    Platform, PlatformError, Renderer, WindowAdapter, WindowEvent,
};
use slint::{LogicalSize, Model, ModelRc, PhysicalSize, VecModel, Window};

use crate::layout::KeyboardLayout;
use crate::layout::key::KeyAction;
use crate::render::engine::RenderEngine;
use crate::wayland::state::HoldPreview;

slint::include_modules!();

/// Icon codes shared with `ui/osk.slint` `Key.icon`.
pub mod icon {
    pub const BACKSPACE: i32 = 1;
    pub const ENTER: i32 = 2;
    pub const SHIFT: i32 = 3;
    pub const WIN: i32 = 4;
    pub const MIC: i32 = 5;
    pub const ARROW_L: i32 = 6;
    pub const ARROW_R: i32 = 7;
    pub const ARROW_U: i32 = 8;
    pub const ARROW_D: i32 = 9;
    pub const GEAR: i32 = 10;
    pub const PALETTE: i32 = 11;
    pub const DISMISS: i32 = 12;
    pub const CLIPBOARD: i32 = 13;
    pub const GRAVE: i32 = 14;
    pub const EURO: i32 = 15;
    pub const STERLING: i32 = 16;
    pub const YEN: i32 = 17;
    pub const CENT: i32 = 18;
    pub const RUPEE: i32 = 19;
    pub const SECTION: i32 = 20;
    pub const PLUSMINUS: i32 = 21;
    pub const MULTIPLY: i32 = 22;
    pub const DIVIDE: i32 = 23;
    pub const NOTEQUAL: i32 = 24;
    pub const DEGREE: i32 = 25;
    pub const BULLET: i32 = 26;
    pub const COPYRIGHT: i32 = 27;
    pub const REGISTERED: i32 = 28;
    pub const TRADEMARK: i32 = 29;
    pub const GUILLEMOTLEFT: i32 = 30;
    pub const GUILLEMOTRIGHT: i32 = 31;
    pub const QUESTIONDOWN: i32 = 32;
    pub const PIN: i32 = 33;
    pub const CAPSLOCK: i32 = 34;
}

const BACKSPACE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#ffffff" d="M22 3H7c-.69 0-1.23.35-1.59.88L0 12l5.41 8.11c.36.53.9.89 1.59.89h15c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm0 16H7.07L2.4 12l4.66-7H22v14zm-11.59-2L14 13.41 17.59 17 19 15.59 15.41 12 19 8.41 17.59 7 14 10.59 10.41 7 9 8.41 12.59 12 9 15.59z"/></svg>"##;
const ENTER_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#ffffff" d="M19 7v4H5.83l3.58-3.59L8 6l-6 6 6 6 1.41-1.41L5.83 13H21V7h-2z"/></svg>"##;
const SHIFT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="none" stroke="#ffffff" stroke-width="2" d="M4 15h6v5h4v-5h6L12 4 4 15z"/></svg>"##;
const CAPSLOCK_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="none" stroke="#ffffff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="M4 14h6v3h4v-3h6L12 3 4 14zm-1 7h18"/></svg>"##;
const WIN_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#ffffff" d="M3 3h8v8H3V3zm10 0h8v8h-8V3zM3 13h8v8H3v-8zm10 0h8v8h-8v-8z"/></svg>"##;
const MIC_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#ffffff" d="M12 14c1.66 0 3-1.34 3-3V5c0-1.66-1.34-3-3-3S9 3.34 9 5v6c0 1.66 1.34 3 3 3zm5-3c0 2.76-2.24 5-5 5s-5-2.24-5-5H5c0 3.53 2.61 6.43 6 6.92V21h2v-2.08c3.39-.49 6-3.39 6-6.92h-2z"/></svg>"##;
const ARROW_L_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#ffffff" d="M15.41 7.41 14 6l-6 6 6 6 1.41-1.41L10.83 12z"/></svg>"##;
const ARROW_R_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#ffffff" d="M8.59 16.59 10 18l6-6-6-6-1.41 1.41L13.17 12z"/></svg>"##;
const ARROW_U_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#ffffff" d="M7.41 15.41 12 10.83l4.59 4.58L18 14l-6-6-6 6z"/></svg>"##;
const ARROW_D_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#ffffff" d="M7.41 8.59 12 13.17l4.59-4.58L18 10l-6 6-6-6z"/></svg>"##;
const GEAR_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#ffffff" d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58c.18-.14.23-.41.12-.61l-1.92-3.32c-.12-.22-.37-.29-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54c-.04-.24-.24-.41-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.58-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z"/></svg>"##;
const PALETTE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#ffffff" d="M12 3c-4.97 0-9 4.03-9 9 0 2.12.74 4.07 1.97 5.61.35.43.91.66 1.46.54.51-.11.89-.52.94-1.04.08-.85.76-1.51 1.63-1.51h1.5c3.03 0 5.5-2.47 5.5-5.5 0-3.92-3.13-7.1-7-7.1zm-4.5 9c-.83 0-1.5-.67-1.5-1.5S6.67 9 7.5 9 9 9.67 9 10.5 8.33 12 7.5 12zm3-4C9.67 8 9 7.33 9 6.5S9.67 5 10.5 5s1.5.67 1.5 1.5S11.33 8 10.5 8zm3 0c-.83 0-1.5-.67-1.5-1.5S12.67 5 13.5 5s1.5.67 1.5 1.5S14.33 8 13.5 8zm3 4c-.83 0-1.5-.67-1.5-1.5S15.67 9 16.5 9s1.5.67 1.5 1.5-.67 1.5-1.5 1.5z"/></svg>"##;
const DISMISS_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"/></svg>"##;
const CLIPBOARD_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#ffffff" d="M19 3h-4.18C14.4 1.84 13.3 1 12 1c-1.3 0-2.4.84-2.82 2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm-7 0c.55 0 1 .45 1 1s-.45 1-1 1-1-.45-1-1 .45-1 1-1zm7 16H5V5h2v2h10V5h2v14z"/></svg>"##;
const GRAVE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M367 410 649 784H496L170 410Z"/></svg>"##;
const EURO_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M1167 670V883Q1076 778 991.5 733.0Q907 688 805 688Q648 688 547.0 788.0Q446 888 414 1075H991L936 1198H398Q396 1222 395.5 1245.0Q395 1268 395 1303Q395 1335 395.5 1358.0Q396 1381 398 1405H844L788 1528H414Q446 1715 547.0 1816.0Q648 1917 805 1917Q907 1917 991.5 1872.0Q1076 1827 1167 1722V1933Q1078 2005 985.5 2041.0Q893 2077 797 2077Q560 2077 405.5 1932.0Q251 1787 211 1528H0L55 1405H194Q194 1382 193.5 1358.5Q193 1335 193 1303Q193 1268 193.5 1244.5Q194 1221 194 1198H0L55 1075H211Q251 818 406.0 673.0Q561 528 797 528Q895 528 987.5 563.5Q1080 599 1167 670Z"/></svg>"##;
const STERLING_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M1102 588V770Q1026 729 958.0 708.5Q890 688 829 688Q681 688 623.0 765.5Q565 843 565 1055V1270H956V1413H565V1878H1122V2048H129V1878H365V1413H166V1270H365V1032Q365 771 472.0 649.5Q579 528 811 528Q872 528 947.5 543.5Q1023 559 1102 588Z"/></svg>"##;
const YEN_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M1165 1593H752V2048H551V1593H135V1470H551V1419L467 1264H135V1141H399L82 555H272L651 1255L1028 555H1219L901 1141H1165V1264H834L750 1419V1470H1165Z"/></svg>"##;
const CENT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M678 1917V1061Q531 1079 449.0 1192.0Q367 1305 367 1489Q367 1674 449.0 1787.0Q531 1900 678 1917ZM1059 971V1143Q985 1102 917.0 1081.0Q849 1060 781 1057L780 1921Q850 1916 918.5 1895.0Q987 1874 1059 1835V2005Q994 2035 925.5 2052.5Q857 2070 780 2077V2361H678V2073Q437 2053 304.5 1899.5Q172 1746 172 1489Q172 1231 304.5 1078.0Q437 925 678 903V616H780L781 903Q854 907 922.5 923.5Q991 940 1059 971Z"/></svg>"##;
const RUPEE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M106 678 161 555H1199L1144 678H790Q869 756 890 885H1199L1144 1008H898Q893 1134 832 1219Q767 1312 642 1348Q707 1370 768 1442Q827 1510 892 1640L1097 2048H880L689 1665Q614 1514 546 1466Q476 1417 356 1417H136V1251H390Q536 1251 610 1184Q678 1122 684 1008H106L161 885H673Q655 818 610 766Q536 678 390 678Z"/></svg>"##;
const SECTION_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M379 1112Q316 1158 285.0 1203.0Q254 1248 254 1294Q254 1370 323.5 1436.5Q393 1503 643 1638Q706 1593 737.0 1547.5Q768 1502 768 1456Q768 1381 696.5 1313.0Q625 1245 379 1112ZM829 586V750Q746 711 674.5 691.5Q603 672 547 672Q450 672 396.0 712.0Q342 752 342 823Q342 913 548 1028Q574 1043 588 1051Q799 1170 864.5 1247.0Q930 1324 930 1425Q930 1515 884.0 1585.0Q838 1655 745 1708Q807 1760 835.5 1814.5Q864 1869 864 1933Q864 2075 762.0 2159.0Q660 2243 487 2243Q414 2243 337.0 2228.5Q260 2214 172 2185V2021Q259 2060 333.0 2079.5Q407 2099 465 2099Q567 2099 623.5 2057.0Q680 2015 680 1939Q680 1837 459 1714L434 1700Q220 1580 156.0 1503.5Q92 1427 92 1325Q92 1234 138.5 1162.5Q185 1091 276 1042Q217 998 187.5 942.0Q158 886 158 817Q158 687 258.0 607.5Q358 528 524 528Q597 528 673.5 542.5Q750 557 829 586Z"/></svg>"##;
const PLUSMINUS_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M942 764V1151H1499V1321H942V1708H774V1321H217V1151H774V764ZM217 1878H1499V2048H217Z"/></svg>"##;
const MULTIPLY_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M1436 948 979 1407 1436 1864 1317 1985 858 1526 399 1985 281 1864 737 1407 281 948 399 827 858 1286 1317 827Z"/></svg>"##;
const DIVIDE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M735 913H981V1159H735ZM735 1653H981V1898H735ZM217 1321H1499V1491H217Z"/></svg>"##;
const NOTEQUAL_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M217 1118H989L1245 803L1370 905L1196 1118H1499V1286H1059L864 1526H1499V1696H725L467 2009L342 1907L516 1696H217V1526H655L850 1286H217Z"/></svg>"##;
const DEGREE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M512 657Q432 657 377.0 712.5Q322 768 322 848Q322 927 377.0 981.5Q432 1036 512 1036Q592 1036 647.0 981.5Q702 927 702 848Q702 769 646.5 713.0Q591 657 512 657ZM512 528Q576 528 635.0 552.5Q694 577 737 623Q783 668 806.0 725.0Q829 782 829 848Q829 980 736.5 1071.5Q644 1163 510 1163Q375 1163 285.0 1073.0Q195 983 195 848Q195 714 287.0 621.0Q379 528 512 528Z"/></svg>"##;
const BULLET_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M307 1286Q307 1162 393.5 1076.5Q480 991 606 991Q730 991 815.5 1076.5Q901 1162 901 1286Q901 1411 815.0 1497.0Q729 1583 604 1583Q479 1583 393.0 1497.0Q307 1411 307 1286Z"/></svg>"##;
const COPYRIGHT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M1024 563Q1176 563 1307.5 618.0Q1439 673 1548 782Q1657 891 1711.0 1022.0Q1765 1153 1765 1307Q1765 1459 1711.0 1589.5Q1657 1720 1548 1829Q1439 1938 1307.5 1993.0Q1176 2048 1024 2048Q872 2048 740.5 1993.0Q609 1938 500 1829Q391 1720 337.0 1589.5Q283 1459 283 1307Q283 1153 337.0 1022.0Q391 891 500 782Q609 673 740.5 618.0Q872 563 1024 563ZM1024 666Q893 666 780.0 713.0Q667 760 573 854Q479 948 431.0 1062.5Q383 1177 383 1307Q383 1436 431.0 1549.5Q479 1663 573 1757Q667 1851 780.0 1898.5Q893 1946 1024 1946Q1156 1946 1269.5 1898.5Q1383 1851 1477 1757Q1570 1664 1616.5 1551.0Q1663 1438 1663 1307Q1663 1174 1616.0 1060.5Q1569 947 1477 854Q1383 760 1269.5 713.0Q1156 666 1024 666ZM1323 911V1040Q1257 1007 1192.0 991.0Q1127 975 1061 975Q912 975 828.5 1062.5Q745 1150 745 1307Q745 1466 830.5 1553.0Q916 1640 1071 1640Q1135 1640 1196.0 1624.5Q1257 1609 1323 1575V1702Q1256 1731 1187.5 1745.0Q1119 1759 1049 1759Q833 1759 707.5 1637.0Q582 1515 582 1307Q582 1098 707.5 977.0Q833 856 1049 856Q1122 856 1190.0 870.0Q1258 884 1323 911Z"/></svg>"##;
const REGISTERED_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M1024 666Q893 666 780.0 713.0Q667 760 573 854Q479 948 431.0 1062.5Q383 1177 383 1307Q383 1436 431.0 1549.5Q479 1663 573 1757Q667 1851 780.0 1898.5Q893 1946 1024 1946Q1156 1946 1269.5 1898.5Q1383 1851 1477 1757Q1570 1664 1616.5 1551.0Q1663 1438 1663 1307Q1663 1174 1616.0 1060.5Q1569 947 1477 854Q1383 760 1269.5 713.0Q1156 666 1024 666ZM1024 563Q1176 563 1307.5 618.0Q1439 673 1548 782Q1657 891 1711.0 1022.0Q1765 1153 1765 1307Q1765 1459 1711.0 1589.5Q1657 1720 1548 1829Q1439 1938 1307.5 1993.0Q1176 2048 1024 2048Q872 2048 740.5 1993.0Q609 1938 500 1829Q391 1720 337.0 1589.5Q283 1459 283 1307Q283 1153 337.0 1022.0Q391 891 500 782Q609 673 740.5 618.0Q872 563 1024 563ZM997 977H874V1253H997Q1107 1253 1150.5 1222.0Q1194 1191 1194 1116Q1194 1040 1150.0 1008.5Q1106 977 997 977ZM1004 874Q1180 874 1267.0 933.5Q1354 993 1354 1114Q1354 1200 1301.5 1256.0Q1249 1312 1153 1329Q1177 1337 1210.5 1375.5Q1244 1414 1290 1487L1427 1710H1255L1126 1501Q1067 1405 1030.5 1379.5Q994 1354 940 1354H874V1710H719V874Z"/></svg>"##;
const TRADEMARK_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M1098 555 1272 811 1436 555H1606V1133H1493V649L1298 952H1243L1040 649V1133H926V555ZM813 555V649H610V1133H496V649H295V555Z"/></svg>"##;
const GUILLEMOTLEFT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M1061 989V1180L760 1448L1061 1716V1907L592 1489V1407ZM627 989V1180L326 1448L627 1716V1907L158 1489V1407Z"/></svg>"##;
const GUILLEMOTRIGHT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M193 989 662 1407V1489L193 1907V1716L494 1448L193 1180ZM627 989 1096 1407V1489L627 1907V1716L928 1448L627 1180Z"/></svg>"##;
const QUESTIONDOWN_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2048 2048"><path fill="#ffffff" d="M500 1329H690V1485Q690 1586 662.5 1651.0Q635 1716 545 1803L455 1891Q397 1944 371.5 1991.0Q346 2038 346 2087Q346 2176 411.5 2231.0Q477 2286 586 2286Q664 2286 754.0 2251.0Q844 2216 940 2149V2337Q846 2394 750.0 2422.0Q654 2450 551 2450Q367 2450 255.0 2353.0Q143 2256 143 2097Q143 2021 179.5 1952.5Q216 1884 305 1798L393 1712Q441 1665 460.0 1638.5Q479 1612 487 1587Q494 1566 497.0 1536.0Q500 1506 500 1452ZM696 1182H494V928H696Z"/></svg>"##;
const PIN_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#ffffff" d="M16 12V4h1V2H7v2h1v8l-2 2v2h5.2v6h1.6v-6H18v-2l-2-2z"/></svg>"##;

fn svg(svg_data: &str) -> slint::Image {
    slint::Image::load_from_svg_data(svg_data.as_bytes()).unwrap()
}

struct Adapter {
    window: Rc<Window>,
    renderer: SoftwareRenderer,
    size: Cell<PhysicalSize>,
}

impl WindowAdapter for Adapter {
    fn window(&self) -> &Window {
        &self.window
    }
    fn size(&self) -> PhysicalSize {
        self.size.get()
    }
    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }
}

fn make_adapter(size: PhysicalSize) -> Rc<Adapter> {
    Rc::<Adapter>::new_cyclic(|weak| Adapter {
        window: Rc::new(Window::new(weak.clone())),
        renderer: SoftwareRenderer::new(),
        size: Cell::new(size),
    })
}

struct Sp {
    adapter: Rc<Adapter>,
}

impl Platform for Sp {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.adapter.clone())
    }
}

#[derive(Clone)]
struct CachedLayout {
    layout: KeyboardLayout,
    width: u32,
    height: u32,
    key_coords: Vec<(usize, usize)>,
}

/// Owns the single Slint platform/adapter/component for the whole daemon.
pub struct SlintScene {
    adapter: Rc<Adapter>,
    ui: OskUi,
    width: u32,
    height: u32,
    key_model: Rc<VecModel<Key>>,
    current_keys: Vec<Key>,
    cached_layout: Option<CachedLayout>,
}

impl SlintScene {
    pub fn new(width: u32, height: u32) -> Result<Self, PlatformError> {
        let size = PhysicalSize::new(width, height);
        let adapter = make_adapter(size);
        slint::platform::set_platform(Box::new(Sp { adapter: adapter.clone() }))
            .map_err(|e| PlatformError::from(e.to_string()))?;

        let ui = OskUi::new()?;
        let icons = ui.global::<OskIcons>();
        icons.set_backspace(svg(BACKSPACE_SVG));
        icons.set_enter(svg(ENTER_SVG));
        icons.set_shift(svg(SHIFT_SVG));
        icons.set_capslock(svg(CAPSLOCK_SVG));
        icons.set_win(svg(WIN_SVG));
        icons.set_mic(svg(MIC_SVG));
        icons.set_arrow_l(svg(ARROW_L_SVG));
        icons.set_arrow_r(svg(ARROW_R_SVG));
        icons.set_arrow_u(svg(ARROW_U_SVG));
        icons.set_arrow_d(svg(ARROW_D_SVG));
        icons.set_gear(svg(GEAR_SVG));
        icons.set_palette(svg(PALETTE_SVG));
        icons.set_dismiss(svg(DISMISS_SVG));
        icons.set_clipboard(svg(CLIPBOARD_SVG));
        icons.set_grave(svg(GRAVE_SVG));
        icons.set_euro(svg(EURO_SVG));
        icons.set_sterling(svg(STERLING_SVG));
        icons.set_yen(svg(YEN_SVG));
        icons.set_cent(svg(CENT_SVG));
        icons.set_rupee(svg(RUPEE_SVG));
        icons.set_section(svg(SECTION_SVG));
        icons.set_plusminus(svg(PLUSMINUS_SVG));
        icons.set_multiply(svg(MULTIPLY_SVG));
        icons.set_divide(svg(DIVIDE_SVG));
        icons.set_notequal(svg(NOTEQUAL_SVG));
        icons.set_degree(svg(DEGREE_SVG));
        icons.set_bullet(svg(BULLET_SVG));
        icons.set_copyright(svg(COPYRIGHT_SVG));
        icons.set_registered(svg(REGISTERED_SVG));
        icons.set_trademark(svg(TRADEMARK_SVG));
        icons.set_guillemotleft(svg(GUILLEMOTLEFT_SVG));
        icons.set_guillemotright(svg(GUILLEMOTRIGHT_SVG));
        icons.set_questiondown(svg(QUESTIONDOWN_SVG));
        icons.set_pin(svg(PIN_SVG));

        let key_model = Rc::new(VecModel::default());
        ui.set_keys(ModelRc::from(key_model.clone()));

        ui.show()?;
        ui.window().dispatch_event(WindowEvent::Resized {
            size: LogicalSize::new(width as f32, height as f32),
        });

        Ok(Self {
            adapter,
            ui,
            width,
            height,
            key_model,
            current_keys: Vec::new(),
            cached_layout: None,
        })
    }

    /// Map a key action (plus label) to the icon code used by `osk.slint`.
    fn icon_for(action: &KeyAction, label: &str) -> i32 {
        match action {
            KeyAction::Backspace => icon::BACKSPACE,
            KeyAction::Enter => icon::ENTER,
            KeyAction::Shift => {
                if label == "⇪" {
                    icon::CAPSLOCK
                } else {
                    icon::SHIFT
                }
            }
            KeyAction::Win => icon::WIN,
            KeyAction::ArrowLeft => icon::ARROW_L,
            KeyAction::ArrowRight => icon::ARROW_R,
            KeyAction::ArrowUp => icon::ARROW_U,
            KeyAction::ArrowDown => icon::ARROW_D,
            KeyAction::Hide => icon::DISMISS,
            KeyAction::Clipboard => icon::CLIPBOARD,
            KeyAction::None if label == "🎤" => icon::MIC,
            KeyAction::None if label == "gear" => icon::GEAR,
            KeyAction::None if label == "palette" => icon::PALETTE,
            KeyAction::Text(t) => match t.as_str() {
                "`" => icon::GRAVE,
                "€" => icon::EURO,
                "£" => icon::STERLING,
                "¥" => icon::YEN,
                "¢" => icon::CENT,
                "₹" => icon::RUPEE,
                "§" => icon::SECTION,
                "±" => icon::PLUSMINUS,
                "×" => icon::MULTIPLY,
                "÷" => icon::DIVIDE,
                "≠" => icon::NOTEQUAL,
                "°" => icon::DEGREE,
                "•" => icon::BULLET,
                "©" => icon::COPYRIGHT,
                "®" => icon::REGISTERED,
                "™" => icon::TRADEMARK,
                "«" => icon::GUILLEMOTLEFT,
                "»" => icon::GUILLEMOTRIGHT,
                "¿" => icon::QUESTIONDOWN,
                _ => 0,
            },
            _ => 0,
        }
    }

    /// Renders `layout` into `out_shm` (an ARGB8888 `wl_shm` canvas).
    ///
    /// Returns `Some(damage_rects)` on success containing the bounding boxes of
    /// regions updated during this frame, or `None` if the Slint rendering step
    /// failed so the caller can fall back to the legacy `RenderEngine`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        layout: &KeyboardLayout,
        theme: &crate::render::theme::Theme,
        width: u32,
        height: u32,
        pressed_keys: &[(usize, usize)],
        latched_keys: &[(usize, usize)],
        _swipe_offset: Option<f32>,
        hold_preview: Option<HoldPreview>,
        out_shm: &mut [u8],
    ) -> Option<Vec<(i32, i32, i32, i32)>> {
        let needs_rebuild = match &self.cached_layout {
            Some(cached) => {
                cached.width != width || cached.height != height || &cached.layout != layout
            }
            None => true,
        };

        if needs_rebuild {
            let rects = RenderEngine::calculate_key_rects(layout, width, height, theme);
            let mut keys = Vec::new();
            let mut key_coords = Vec::new();

            for (r_idx, row) in rects.iter().enumerate() {
                for (rect, key_idx) in row.iter() {
                    let k_idx = *key_idx;
                    let Some(key) = layout.rows.get(r_idx).and_then(|r| r.keys.get(k_idx)) else {
                        continue;
                    };
                    let icon = Self::icon_for(&key.action, &key.label);
                    let is_space = matches!(key.action, KeyAction::Space);
                    let is_active = pressed_keys.contains(&(r_idx, k_idx)) || latched_keys.contains(&(r_idx, k_idx));

                    let key_type = if key.is_clipboard {
                        if key.label == "Clipboard History" {
                            KeyType::ClipboardHeader
                        } else if key.label == "◀ Back" {
                            KeyType::ClipboardBack
                        } else {
                            KeyType::ClipboardItem
                        }
                    } else if key.is_suggestion {
                        KeyType::Suggestion
                    } else if is_space {
                        KeyType::Space
                    } else {
                        KeyType::Standard
                    };

                    let label_str = if icon > 0 || is_space {
                        ""
                    } else {
                        key.label.as_str()
                    };

                    keys.push(Key {
                        x: rect.x,
                        y: rect.y,
                        w: rect.w,
                        h: rect.h,
                        label: label_str.into(),
                        sub: key.secondary_label.clone().unwrap_or_default().into(),
                        icon,
                        key_type,
                        is_pressed: is_active,
                        is_functional: key.is_special,
                        is_pinned: key.is_pinned,
                    });
                    key_coords.push((r_idx, k_idx));
                }
            }

            let show_clipboard = keys.iter().any(|k| {
                matches!(
                    k.key_type,
                    KeyType::ClipboardHeader | KeyType::ClipboardBack | KeyType::ClipboardItem
                )
            });
            self.ui.set_show_clipboard(show_clipboard);

            self.current_keys = keys.clone();
            self.key_model.set_vec(keys);
            self.cached_layout = Some(CachedLayout {
                layout: layout.clone(),
                width,
                height,
                key_coords,
            });
        } else {
            // Layout geometry is unchanged. Zero allocations: only update is_pressed for changed keys.
            let key_coords = &self.cached_layout.as_ref().unwrap().key_coords;
            for (flat_idx, &(r_idx, k_idx)) in key_coords.iter().enumerate() {
                let is_active = pressed_keys.contains(&(r_idx, k_idx)) || latched_keys.contains(&(r_idx, k_idx));
                if self.current_keys[flat_idx].is_pressed != is_active {
                    self.current_keys[flat_idx].is_pressed = is_active;
                    self.key_model.set_row_data(flat_idx, self.current_keys[flat_idx].clone());
                }
            }
        }

        // Let animations/timers tick before rendering the frame.
        slint::platform::update_timers_and_animations();

        if let Some(preview) = hold_preview {
            self.ui.set_hold_preview_visible(true);
            self.ui.set_hold_preview_label(preview.label.into());
            self.ui.set_hold_preview_x(preview.x);
            self.ui.set_hold_preview_y(preview.y);
            self.ui.set_hold_preview_w(preview.w);
            self.ui.set_hold_preview_h(preview.h);
        } else if self.ui.get_hold_preview_visible() {
            self.ui.set_hold_preview_visible(false);
        }

        if width != self.width || height != self.height {
            self.width = width;
            self.height = height;
            self.adapter.size.set(PhysicalSize::new(width, height));
            self.ui.window().dispatch_event(WindowEvent::Resized {
                size: LogicalSize::new(width as f32, height as f32),
            });
        }

        let line_provider = ShmLineBufferProvider {
            target: out_shm,
            width: width as usize,
            height: height as usize,
            scratch: Vec::new(),
        };
        let dirty_region = self.adapter.renderer.render_by_line(line_provider);
        let damage_rects: Vec<(i32, i32, i32, i32)> = dirty_region
            .iter()
            .map(|(pos, sz)| (pos.x, pos.y, sz.width as i32, sz.height as i32))
            .collect();
        Some(damage_rects)
    }
}

/// Zero-copy adapter implementing Slint's `LineBufferProvider`.
///
/// Rather than allocating an entire frame-sized buffer (`width * height * 4` bytes)
/// in RAM and doing a secondary full-frame traversal, this provider renders
/// line-by-line using a small scratch buffer of `width` pixels, translating each
/// line directly into the Wayland `wl_shm` ARGB8888 buffer.
struct ShmLineBufferProvider<'a> {
    target: &'a mut [u8],
    width: usize,
    height: usize,
    scratch: Vec<PremultipliedRgbaColor>,
}

impl<'a> slint::platform::software_renderer::LineBufferProvider for ShmLineBufferProvider<'a> {
    type TargetPixel = PremultipliedRgbaColor;

    fn process_line(
        &mut self,
        line: usize,
        range: core::ops::Range<usize>,
        render_fn: impl FnOnce(&mut [Self::TargetPixel]),
    ) {
        if line >= self.height {
            return;
        }

        let needed_len = range.end - range.start;
        if self.scratch.len() < needed_len {
            self.scratch.resize(needed_len, PremultipliedRgbaColor::default());
        }

        let scratch_slice = &mut self.scratch[..needed_len];
        render_fn(scratch_slice);

        let line_offset = (line * self.width + range.start) * 4;
        let dst_bytes = &mut self.target[line_offset..line_offset + needed_len * 4];

        for (dst_chunk, src_px) in dst_bytes.chunks_exact_mut(4).zip(scratch_slice.iter()) {
            dst_chunk[0] = src_px.blue;
            dst_chunk[1] = src_px.green;
            dst_chunk[2] = src_px.red;
            dst_chunk[3] = src_px.alpha;
        }
    }
}