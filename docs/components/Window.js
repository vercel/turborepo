/* eslint-disable react/prop-types */
import React from "react";

export class Window extends React.PureComponent {
  render() {
    const {
      titleColor,
      backgroundColor,
      borderColor,
      shadowColor,
      titleBg,
      white,
      alias,
      className,
      mini,
      caption = null,
    } = this.props;

    const height = null != this.props.height ? `${this.props.height}px` : null;
    const noScroll = false === this.props.scroll;

    const classes = ["window", "terminal-special"];

    if (white) {
      classes.push("white");
    }

    if (alias) {
      classes.push("alias");
    }

    if (mini) {
      classes.push("mini");
    }

    return (
      <div
        className={[...classes, className].join(" ")}
        style={{ backgroundColor, borderColor, height }}
      >
        <div className="header ">
          <span className="icon close" />
          <span className="icon minimize" />
          <span className="icon fullScreen" />
          <div
            className={"title " + (titleBg ? "title-bg" : "")}
            style={{ color: titleColor }}
          >
            {this.props.title}
          </div>
        </div>

        <div className={"body " + (noScroll ? "noScroll" : "")}>
          {this.props.children}
        </div>

        {caption && <p className="caption">{caption}</p>}

        <style jsx>
          {`
            .window {
              width: 100%;
              height: 100%;
              background: #050b13;
              position: relative;
              border-radius: 8px;
            }
            .window.white {
              border: 0;
              box-shadow: 0 20px 50px 0 ${shadowColor || "rgba(0, 0, 0, 0.1)"};
              background: #fff;
              color: #000
            }
            .window.alias {
              box-shadow: none;
            }
            .window.mini {
              background: #050b13;
            }
            .window.mini.white {
              background: #fff;
            }
            .header {
              width: 100%;
              height: 36px;
              position: absolute;
            }
            .body {
              width: 100%;
              height: 100%;
              border-radius: 5px;
              padding-top: 38px;
              padding-left:10px;
              padding-right:10px;
              position: absolute;
            }
            .terminal-special {
              position: relative;
              z-index: 1;
              border: none;
              transform-style: preserve-3d;
              transition: background 350ms ease-in-out, color 350ms ease-in-out;
              cursor: pointer;
              display: inline-block;
              white-space: nowrap;
              min-width: 28px;
              border-radius: 8px;
            }

            .terminal-special:after {
              content: '';
              display: block;
              position: absolute;
              top: -1px;
              left: -1px;
              width: calc(100% + 2.5px);
              height: calc(100% + 2px);
              background: linear-gradient(to right, #ef4444 10%, #3b82f6 90%);
              background-size: 100% 200%;
              transform: translateZ(-1px);
              border-radius: 8px;
              overflow: hidden;
            }
            .window.mini .body {
              padding-top: 20px;
            }
            .body.noScroll {
              overflow: hidden;
            }
            .icon {
              border-radius: 50%;
              display: inline-block;
              width: 12px;
              height: 12px;
              position: absolute;
              top: 52%;
              transform: translate3d(0px, -50%, 0px);
            }
            .mini .icon {
              top: 30%;
            }
            .close {
              background-color: #ff5f56;
              left: 13px;
            }
            .minimize {
              background-color: #ffbd2e;
              left: 33px;
            }
            .fullScreen {
              background-color: #27c93f;
              left: 53px;
            }
            .mini .icon {
              width: 7px;
              height: 7px;
            }
            .mini .close {
              left: 7px;
            }
            .mini .minimize {
              left: 18px;
            }
            .mini .fullScreen {
              left: 30px;
            }
            .title {
              color: #999;
              font-size: 12px;
              font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI',
                'Roboto', 'Oxygen', 'Ubuntu', 'Cantarell', 'Fira Sans',
                'Droid Sans', 'Helvetica Neue', sans-serif;
              position: absolute;
              top: 6px;
              left: 0;
              right: 0;
              text-align: center;
              width: 200px;
              margin: auto;
              border-radius: 4px;
              padding: 4px;
            }
            .title-bg {
              background-color: #f7f7f7;
            }
            .mini .title {
              display: none;
            }
            .caption {
              color: #999;
              font-size: 12px;
              margin: 0;
              text-align: center;
              position: absolute;
              bottom: -50px;
              width: 100%;
            }
            @media (max-width: 350px) {
              .caption {
                bottom: -60px;
                padding: 0 20px;
              }
            }
          `}
        </style>
      </div>
    );
  }
}
