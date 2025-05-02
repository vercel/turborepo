import Link from "next/link";
import Image from "next/image";

interface CardProps {
  name: string;
  alias: string;
  avatar: string;
  url: string;
  children: React.ReactNode;
}

export const Card = ({ name, alias, avatar, url, children }: CardProps) => {
  return (
    <Link
      href={url}
      target="_blank"
      className="drop-shadow-xs rounded-xl border border-gray-400 bg-background-100 px-6 py-5 hover:bg-background-200 hover:drop-shadow-sm"
    >
      <div className="flex items-center gap-4">
        <Image
          src={avatar}
          alt={name}
          width={40}
          height={40}
          className="h-12 w-12 rounded-full bg-gray-200"
        />
        <div className="flex flex-col">
          <div className="font-medium text-gray-1000 text-label-16">{name}</div>
          <div className="text-gray-800 text-copy-16">{alias}</div>
        </div>
      </div>
      <div className="mt-4 text-gray-1000 text-copy-16">{children}</div>
    </Link>
  );
};

export const Testimonials = () => {
  return (
    <div className="mb-6 mt-7 grid grid-cols-1 gap-4 min-[780px]:grid-cols-3">
      <div className="grid gap-4">
        <Card
          name="Matt Pocock"
          alias="@mattpocockuk"
          avatar="https://pbs.twimg.com/profile_images/1666460461884211204/SmBm505D_400x400.jpg"
          url="https://x.com/mattpocockuk/status/1498696992943452168"
        >
          <p className="mb-4">
            🤯 @turborepo saved us 67 HOURS of CI since we adopted it.
          </p>
          <p>That's a team of only 4 full-time devs at @statelyai. Nuts.</p>
        </Card>
        <Card
          name="Lewis ⚡"
          alias="@lewisbuildsai"
          avatar="https://pbs.twimg.com/profile_images/1873766582855282688/OXiFZXAY_400x400.jpg"
          url="https://x.com/lewisbuildsai/status/1906014322926420104"
        >
          <p className="mb-4">
            If turborepo has a million fans, then I am one of them. If turborepo
            has ten fans, then I am one of them. If turborepo has only one fan
            then that is me. If turborepo has no fans, then that means I am no
            longer on earth. If the world is against turborepo, then I am
            against the world.
          </p>
        </Card>
      </div>

      <div className="grid gap-4">
        <Card
          name="Cory House"
          alias="@housecor"
          avatar="https://pbs.twimg.com/profile_images/1869130199045906432/PUA1SYIL_400x400.jpg"
          url="https://x.com/housecor/status/1580923528596946944"
        >
          <p className="mb-4">
            I’m really enjoying @turborepo. It’s a game changer.
          </p>

          <p className="mb-4">
            With Turbo, I have an incentive to break my repo down into many
            small, focused packages. This way, Turbo can cache each little
            build. So when I hit save, only the package I changed needs to
            recompile.
          </p>
          <p>Really speeds things up!</p>
        </Card>
        <Card
          name="Shrey Gupta"
          alias="@shreygups"
          avatar="https://pbs.twimg.com/profile_images/1783636965306585088/DIgF_W5I_400x400.jpg"
          url="https://x.com/shreygups/status/1900284978367520817"
        >
          <p>wait i kinda love turborepo</p>
        </Card>
      </div>
      <div className="grid gap-4">
        <Card
          name="ALIAS"
          alias="LoadingALIAS"
          avatar="https://pbs.twimg.com/profile_images/1675186185000431619/0NWH60bL_400x400.jpg"
          url="https://x.com/LoadingALIAS/status/1903228609021477283"
        >
          <p>Turborepo is the best.</p>
        </Card>
        <Card
          name="Andrew Lisowski"
          alias="@HipsterSmoothie"
          avatar="https://pbs.twimg.com/profile_images/1685360377754947584/PhKnYmq-_400x400.jpg"
          url="https://x.com/HipsterSmoothie/status/1829245704386269515"
        >
          <p>
            Just setting up @turborepo in @DescriptApp's front-end cut our bill
            in half and saved us $20k
          </p>
        </Card>
        <Card
          name="Pontus Abrahamsson"
          alias="@pontusab"
          avatar="https://pbs.twimg.com/profile_images/1755611130368770048/JwLEqyeo_400x400.jpg"
          url="https://x.com/pontusab/status/1827264818765799931"
        >
          <p>
            I love a monorepo setup, having everything in one place powered by
            Turborepo 🔥
          </p>
        </Card>
      </div>
    </div>
  );
};
