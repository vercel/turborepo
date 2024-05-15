import {
  Column,
  CreateDateColumn,
  Entity,
  PrimaryGeneratedColumn,
  UpdateDateColumn,
} from "typeorm";

@Entity({
  name: "todo",
})
export class Todo {
  @PrimaryGeneratedColumn()
  id: number;

  @Column({
    nullable: false,
    comment: "내용",
    length: 100,
  })
  content: string;

  @Column()
  complete: boolean;

  @CreateDateColumn()
  createdAt: string;

  @UpdateDateColumn()
  updatedAt: string;
}
