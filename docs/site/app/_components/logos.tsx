import { cn } from "#components/cn.ts";

export function VercelLogo({ className }: { className?: string }): JSX.Element {
  return (
    <svg
      className={cn(className, "fill-black dark:fill-white")}
      fill="none"
      height={22}
      viewBox="0 0 235 203"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M117.082 0L234.164 202.794H0L117.082 0Z" fill="currentColor" />
    </svg>
  );
}

export function NodeJsLogo({ className }: { className: string }): JSX.Element {
  return (
    <svg
      className={className}
      fill="none"
      height="16"
      strokeLinejoin="round"
      viewBox="0 0 16 16"
      width="16"
      xmlns="http://www.w3.org/2000/svg"
    >
      <mask
        height="16"
        id="mask0_872_3158"
        maskUnits="userSpaceOnUse"
        width="14"
        x="1"
        y="0"
      >
        <path
          d="M7.62322 0.101215L1.37744 3.72072C1.1435 3.85617 1 4.10623 1 4.37653V11.6206C1 11.8911 1.1435 12.141 1.37744 12.2764L7.62367 15.8987C7.85716 16.0338 8.14506 16.0338 8.37826 15.8987L14.6234 12.2764C14.8562 12.141 15 11.8909 15 11.6206V4.37653C15 4.10623 14.8562 3.85617 14.622 3.72072L8.37767 0.101215C8.26055 0.0337871 8.13009 0 7.99963 0C7.86917 0 7.73871 0.0337871 7.62159 0.101215"
          fill="white"
        />
      </mask>
      <g mask="url(#mask0_872_3158)">
        <path
          d="M21.3115 3.10613L3.71197 -5.55525L-5.31201 12.9276L12.2871 21.5894L21.3115 3.10613Z"
          fill="url(#paint0_linear_872_3158)"
        />
      </g>
      <mask
        height="16"
        id="mask1_872_3158"
        maskUnits="userSpaceOnUse"
        width="14"
        x="1"
        y="0"
      >
        <path
          d="M1.15454 12.0805C1.21429 12.1584 1.289 12.2258 1.37692 12.2764L6.73468 15.3836L7.62714 15.8986C7.76057 15.976 7.91267 16.0087 8.06211 15.9976C8.11192 15.9936 8.16173 15.9842 8.21036 15.9703L14.7977 3.86019C14.7473 3.80511 14.6883 3.75897 14.6222 3.72027L10.5325 1.34915L8.37077 0.100323C8.30939 0.0646001 8.24282 0.0392964 8.17507 0.0214348L1.15454 12.0805Z"
          fill="white"
        />
      </mask>
      <g mask="url(#mask1_872_3158)">
        <path
          d="M-6.45459 5.66793L5.97248 22.555L22.4075 10.3636L9.97968 -6.52305L-6.45459 5.66793Z"
          fill="url(#paint1_linear_872_3158)"
        />
      </g>
      <mask
        height="16"
        id="mask2_872_3158"
        maskUnits="userSpaceOnUse"
        width="14"
        x="1"
        y="0"
      >
        <path
          d="M7.92494 0.00417044C7.82013 0.0145897 7.71769 0.0473349 7.62325 0.101217L1.39526 3.7103L8.11099 15.9916C8.20439 15.9782 8.29631 15.947 8.37933 15.8988L14.6251 12.2764C14.8178 12.1642 14.9498 11.9743 14.9898 11.759L8.14361 0.0165236C8.0932 0.00655088 8.0428 0.00134277 7.99091 0.00134277C7.97016 0.00134277 7.9494 0.00238306 7.92865 0.00431807"
          fill="white"
        />
      </mask>
      <g mask="url(#mask2_872_3158)">
        <path
          d="M1.39502 0.00134277V15.9919H14.987V0.00134277H1.39502Z"
          fill="url(#paint2_linear_872_3158)"
        />
      </g>
      <defs>
        <linearGradient
          gradientUnits="userSpaceOnUse"
          id="paint0_linear_872_3158"
          x1="12.5064"
          x2="3.42452"
          y1="-1.23818"
          y2="17.2146"
        >
          <stop offset="0.3" stopColor="#3E863D" />
          <stop offset="0.5" stopColor="#55934F" />
          <stop offset="0.8" stopColor="#5AAD45" />
        </linearGradient>
        <linearGradient
          gradientUnits="userSpaceOnUse"
          id="paint1_linear_872_3158"
          x1="-0.166585"
          x2="16.3156"
          y1="14.2083"
          y2="2.07868"
        >
          <stop offset="0.57" stopColor="#3E863D" />
          <stop offset="0.72" stopColor="#619857" />
          <stop offset="1" stopColor="#76AC64" />
        </linearGradient>
        <linearGradient
          gradientUnits="userSpaceOnUse"
          id="paint2_linear_872_3158"
          x1="1.39961"
          x2="14.9896"
          y1="7.99708"
          y2="7.99708"
        >
          <stop offset="0.16" stopColor="#6BBF47" />
          <stop offset="0.38" stopColor="#79B461" />
          <stop offset="0.47" stopColor="#75AC64" />
          <stop offset="0.7" stopColor="#659E5A" />
          <stop offset="0.9" stopColor="#3E863D" />
        </linearGradient>
      </defs>
    </svg>
  );
}

export function TurborepoLogo({
  className,
}: {
  className?: string;
}): JSX.Element {
  return (
    <svg
      className={className}
      aria-label="Turborepo logomark"
      height="80"
      role="img"
      viewBox="0 0 40 40"
      width="80"
    >
      <path
        className="fill-black dark:fill-white"
        d="M19.9845 6.99291C12.818 6.99291 6.98755 12.8279 6.98755 19.9999C6.98755 27.1721 12.818 33.0071 19.9845 33.0071C27.1509 33.0071 32.9814 27.1721 32.9814 19.9999C32.9814 12.8279 27.1509 6.99291 19.9845 6.99291ZM19.9845 26.7313C16.2694 26.7313 13.2585 23.718 13.2585 19.9999C13.2585 16.282 16.2694 13.2687 19.9845 13.2687C23.6996 13.2687 26.7105 16.282 26.7105 19.9999C26.7105 23.718 23.6996 26.7313 19.9845 26.7313Z"
        fill="white"
      ></path>
      <path
        clip-rule="evenodd"
        d="M21.0734 4.85648V0C31.621 0.564369 40 9.30362 40 19.9999C40 30.6963 31.621 39.4332 21.0734 40V35.1435C28.9344 34.5815 35.1594 28.0078 35.1594 19.9999C35.1594 11.9922 28.9344 5.41843 21.0734 4.85648ZM8.52181 29.931C6.43794 27.5233 5.09469 24.4568 4.85508 21.09H0C0.251709 25.8011 2.13468 30.0763 5.08501 33.368L8.51938 29.931H8.52181ZM18.8951 40V35.1435C15.5285 34.9037 12.4644 33.5619 10.0587 31.4739L6.62435 34.9109C9.91593 37.866 14.1876 39.7481 18.8927 40H18.8951Z"
        fill="url(#paint0_linear_902_224)"
        fill-rule="evenodd"
      ></path>
      <defs>
        <linearGradient
          gradientUnits="userSpaceOnUse"
          id="#paint0_linear_902_224"
          x1="21.8576"
          x2="2.17018"
          y1="2.81244"
          y2="22.4844"
        >
          <stop stop-color="#0096FF"></stop>
          <stop offset="1" stop-color="#FF1E56"></stop>
        </linearGradient>
        <mask id="logo-mask">
          <rect
            fill="url(#gradient)"
            height="80"
            transform="translate(-8,0)"
            width="80"
            x="0"
            y="0"
          />
        </mask>
      </defs>
    </svg>
  );
}

export function NextJSLogo({ className }: { className?: string }): JSX.Element {
  return (
    <svg
      className={className}
      fill="none"
      height="180"
      viewBox="0 0 180 180"
      width="180"
      xmlns="http://www.w3.org/2000/svg"
    >
      <mask
        height="180"
        id="mask0_408_139"
        maskUnits="userSpaceOnUse"
        style={{ maskType: "alpha" }}
        width="180"
        x="0"
        y="0"
      >
        <circle cx="90" cy="90" fill="black" r="90" />
      </mask>
      <g mask="url(#mask0_408_139)">
        <circle
          cx="90"
          cy="90"
          fill="black"
          r="87"
          stroke="white"
          strokeWidth="6"
        />
        <path
          d="M149.508 157.52L69.142 54H54V125.97H66.1136V69.3836L139.999 164.845C143.333 162.614 146.509 160.165 149.508 157.52Z"
          fill="url(#paint0_linear_408_139)"
        />
        <rect
          fill="url(#paint1_linear_408_139)"
          height="72"
          width="12"
          x="115"
          y="54"
        />
      </g>
      <defs>
        <linearGradient
          gradientUnits="userSpaceOnUse"
          id="paint0_linear_408_139"
          x1="109"
          x2="144.5"
          y1="116.5"
          y2="160.5"
        >
          <stop stopColor="white" />
          <stop offset="1" stopColor="white" stopOpacity="0" />
        </linearGradient>
        <linearGradient
          gradientUnits="userSpaceOnUse"
          id="paint1_linear_408_139"
          x1="121"
          x2="120.799"
          y1="54"
          y2="106.875"
        >
          <stop stopColor="white" />
          <stop offset="1" stopColor="white" stopOpacity="0" />
        </linearGradient>
      </defs>
    </svg>
  );
}

export function DesignSystemLogo({
  className,
}: {
  className?: string;
}): JSX.Element {
  return (
    <svg
      className={className}
      fill="none"
      height="24"
      shapeRendering="geometricPrecision"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.5"
      viewBox="0 0 24 24"
      width="24"
    >
      <path
        clipRule="evenodd"
        d="M9.67025 6.612L9.09631 7.63233L10.4037 8.36772L10.9772 7.34818C10.4872 7.1987 10.0429 6.94464 9.67025 6.612ZM13.0228 7.34818L13.5963 8.36772L14.9037 7.63233L14.3297 6.612C13.9571 6.94464 13.5128 7.1987 13.0228 7.34818ZM6.41944 20.75C6.4722 20.5084 6.49999 20.2574 6.49999 20C6.49999 19.7426 6.4722 19.4916 6.41945 19.25H7.49999V20.75H6.41944ZM5.32976 17.388L5.90367 16.3677L4.59631 15.6323L4.02284 16.6518C4.51277 16.8013 4.95708 17.0554 5.32976 17.388ZM17.5805 19.25C17.5278 19.4916 17.5 19.7426 17.5 20C17.5 20.2574 17.5278 20.5084 17.5805 20.75H16.5V19.25H17.5805ZM19.9771 16.6518C19.4872 16.8013 19.0429 17.0554 18.6702 17.388L18.0963 16.3677L19.4037 15.6323L19.9771 16.6518ZM9.50367 9.96772L8.60367 11.5677L7.29631 10.8323L8.19631 9.23233L9.50367 9.96772ZM7.70367 13.1677L6.80367 14.7677L5.49631 14.0323L6.39631 12.4323L7.70367 13.1677ZM15.3963 11.5677L14.4963 9.96772L15.8037 9.23233L16.7037 10.8323L15.3963 11.5677ZM17.1963 14.7677L16.2963 13.1677L17.6037 12.4323L18.5037 14.0323L17.1963 14.7677ZM12.9 20.75H14.7V19.25H12.9V20.75ZM9.29999 20.75H11.1V19.25H9.29999V20.75Z"
        fill="currentColor"
        fillRule="evenodd"
        strokeWidth="0"
      />
      <circle cx="12" cy="4" r="2" stroke="currentColor" strokeWidth="1.5" />
      <circle cx="3" cy="20" r="2" stroke="currentColor" strokeWidth="1.5" />
      <circle cx="21" cy="20" r="2" stroke="currentColor" strokeWidth="1.5" />
    </svg>
  );
}

export function GithubLogo({ className }: { className?: string }): JSX.Element {
  return (
    <svg
      className={className}
      height="24"
      shapeRendering="geometricPrecision"
      viewBox="0 0 24 24"
      width="24"
    >
      <path
        d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12"
        fill="currentColor"
      />
    </svg>
  );
}

export function IconType({ className }: { className?: string }): JSX.Element {
  return (
    <svg
      className={className}
      fill="none"
      height="24"
      shapeRendering="geometricPrecision"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="1.5"
      viewBox="0 0 24 24"
      width="24"
    >
      <path d="M4 7V4h16v3" />
      <path d="M9 20h6" />
      <path d="M12 4v16" />
    </svg>
  );
}
